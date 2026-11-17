//! `Proxy`: an object whose every action can be handled by someone else's functions.

use crate::object::*;
use crate::text::{self, Atom};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

pub(crate) struct ProxyData {
    pub(crate) target: Value,
    pub(crate) handler: Value,
    pub(crate) callable: bool,
    /// Whether it's a module's namespace, which answers for itself: see [`Vm::new_namespace`].
    pub(crate) namespace: bool,
}

impl Vm {
    pub(crate) fn proxy_of(&self, object: u32) -> Option<(Value, Value)> {
        match &self.objects[object as usize].kind {
            Kind::Proxy(data) => Some((data.target, data.handler)),
            _ => None,
        }
    }

    /// What the handler has for this action, if anything. A revoked proxy can't do anything at all.
    fn trap(&mut self, handler: Value, name: Atom) -> Run<Option<Value>> {
        if !handler.is_object() {
            let name = self.atom_text(name);
            return Err(self.type_error(&format!(
                "Cannot perform '{name}' on a proxy that has been revoked"
            )));
        }
        let function = self.trap_function(handler, name)?;
        if function.is_nullish() {
            return Ok(None);
        }
        if !self.is_callable(function) {
            let name = self.atom_text(name);
            let shown = self.brief(function);
            return Err(self.type_error(&format!(
                "{shown} returned for property '{name}' of object '#<Object>' is not a function"
            )));
        }
        Ok(Some(function))
    }

    /// `handler[name]`, from where it was found last time for a plain handler of the same shape that has it itself, as a
    /// handler is usually an object literal of methods that every proxy of a kind shares, read once for each action.
    #[inline]
    fn trap_function(&mut self, handler: Value, name: Atom) -> Run<Value> {
        let object = handler.handle();
        let found = &self.objects[object as usize];
        if !matches!(found.kind, Kind::Plain) {
            return self.get(handler, name);
        }
        let shape = found.shape;
        let place = (shape ^ name.0) as usize % self.trap_cache.len();
        let (known_shape, known_name, slot) = self.trap_cache[place];
        if known_shape == shape && known_name == name.0 {
            return Ok(self.objects[object as usize].slots()[slot as usize]);
        }
        if !self.shapes.is_dict(shape)
            && let Some((slot, flags)) = self.shapes.lookup(shape, name)
            && flags & ACCESSOR == 0
        {
            self.trap_cache[place] = (shape, name.0, slot);
            return Ok(self.objects[object as usize].slots()[slot as usize]);
        }
        self.get(handler, name)
    }

    pub(crate) fn proxy_get(&mut self, object: u32, key: Atom, receiver: Value) -> Run<Value> {
        if let Some(target) = self.namespace_target(Value::object(object)) {
            return self.namespace_get(target, key);
        }
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        match self.trap(handler, text::GET)? {
            Some(trap) => {
                let key = self.atom_value(key);
                self.call(trap, handler, &[target, key, receiver])
            }
            None => self.get_from(target.handle(), key, receiver),
        }
    }

    /// [`Vm::proxy_get`] for a read by the interpreter, whose answer goes to register `dst` of the frame running. A
    /// `get` trap written in JavaScript is called in place: its frame goes on top, answering to `dst`, and this fails
    /// with [`Value::ENTERED`], so the interpreter carries on in it, as it does for a call, rather than Rust starting
    /// the interpreter again for it. Anything else is as `proxy_get`.
    #[inline(never)]
    pub(crate) fn proxy_get_here(&mut self, object: u32, key: Atom, dst: u16) -> Run<Value> {
        let receiver = Value::object(object);
        if let Some(target) = self.namespace_target(receiver) {
            return self.namespace_get(target, key);
        }
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        let Some(trap) = self.trap(handler, text::GET)? else {
            return self.get_from(target.handle(), key, receiver);
        };
        let key = self.atom_value(key);
        if let Kind::Function { proto, .. } = self.objects[trap.index()].kind
            && self.protos[proto as usize].flags & crate::op::IS_CLASS == 0
        {
            // As a call puts them: what it's called on, then its arguments.
            let base = self.top;
            self.reserve_stack(base + 4 + 16)?;
            self.stack[base] = handler;
            self.stack[base + 1] = target;
            self.stack[base + 2] = key;
            self.stack[base + 3] = receiver;
            self.enter_function(trap.handle(), proto, base, 3, dst, 0, Value::UNDEFINED)?;
            return Err(Value::ENTERED);
        }
        self.call(trap, handler, &[target, key, receiver])
    }

    pub(crate) fn proxy_set(
        &mut self,
        object: u32,
        key: Atom,
        value: Value,
        receiver: Value,
    ) -> Run<()> {
        if self.namespace_target(Value::object(object)).is_some() {
            return Err(self.namespace_refuses("set", key));
        }
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        match self.trap(handler, text::SET)? {
            Some(trap) => {
                let key_value = self.atom_value(key);
                let result = self.call(trap, handler, &[target, key_value, value, receiver])?;
                if !self.to_boolean(result) {
                    let name = self.atom_text(key);
                    return Err(self.type_error(&format!(
                        "'set' on proxy: trap returned falsish for property '{name}'"
                    )));
                }
                Ok(())
            }
            None => self.set_on(target.handle(), key, value),
        }
    }

    pub(crate) fn proxy_has(&mut self, object: u32, key: Atom) -> Run<bool> {
        if let Some(target) = self.namespace_target(Value::object(object)) {
            return Ok(self.namespace_has(target, key));
        }
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        match self.trap(handler, text::TRAP_HAS)? {
            Some(trap) => {
                let key = self.atom_value(key);
                let result = self.call(trap, handler, &[target, key])?;
                Ok(self.to_boolean(result))
            }
            None => self.has_property_checked(target.handle(), key),
        }
    }

    pub(crate) fn proxy_delete(&mut self, object: u32, key: Atom) -> Run<bool> {
        if let Some(target) = self.namespace_target(Value::object(object)) {
            if self.namespace_has(target, key) {
                return Err(self.namespace_refuses("delete", key));
            }
            return Ok(true);
        }
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        match self.trap(handler, text::TRAP_DELETE_PROPERTY)? {
            Some(trap) => {
                let key_value = self.atom_value(key);
                let result = self.call(trap, handler, &[target, key_value])?;
                if !self.to_boolean(result) {
                    let name = self.atom_text(key);
                    return Err(self.type_error(&format!(
                        "'deleteProperty' on proxy: trap returned falsish for property '{name}'"
                    )));
                }
                Ok(true)
            }
            None => self.delete(target.handle(), key),
        }
    }

    /// The keys and their flags. The flags come from asking about each key, as a handler can say anything.
    pub(crate) fn proxy_own_keys(&mut self, object: u32) -> Run<Vec<(Atom, u8)>> {
        if let Some(target) = self.namespace_target(Value::object(object)) {
            return Ok(self.namespace_keys(target));
        }
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        let Some(trap) = self.trap(handler, text::TRAP_OWN_KEYS)? else {
            let keys = self.own_keys_checked(target.handle())?;
            // The handler still gets to say what each one's like.
            if self.trap(handler, text::TRAP_GET_OWN_PROPERTY_DESCRIPTOR)?.is_none() {
                return Ok(keys);
            }
            let mut out = Vec::with_capacity(keys.len());
            for (key, _) in keys {
                out.push((key, self.proxy_key_flags(object, key)?));
            }
            return Ok(out);
        };
        let result = self.call(trap, handler, &[target])?;
        let length = self.length_of(result)?;
        let mut out = Vec::with_capacity(length);
        for index in 0..length {
            let item = self.get_index(result, index)?;
            if !item.is_string() && !item.is_symbol() {
                return Err(self.type_error("A proxy's ownKeys must give strings and symbols"));
            }
            let key = self.to_atom(item)?;
            out.push((key, self.proxy_key_flags(object, key)?));
        }
        Ok(out)
    }

    fn proxy_key_flags(&mut self, object: u32, key: Atom) -> Run<u8> {
        let descriptor = self.descriptor_checked(object, key)?;
        if !descriptor.is_object() {
            return Ok(0);
        }
        let mut flags = 0;
        for (name, bit) in [
            (crate::text::ENUMERABLE, ENUMERABLE),
            (crate::text::CONFIGURABLE, CONFIGURABLE),
            (crate::text::WRITABLE, WRITABLE),
        ] {
            let value = self.get(descriptor, name)?;
            if self.to_boolean(value) {
                flags |= bit;
            }
        }
        Ok(flags)
    }

    pub(crate) fn proxy_descriptor(&mut self, object: u32, key: Atom) -> Run<Value> {
        if let Some(target) = self.namespace_target(Value::object(object)) {
            return self.namespace_descriptor(target, key);
        }
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        match self.trap(handler, text::TRAP_GET_OWN_PROPERTY_DESCRIPTOR)? {
            Some(trap) => {
                let key = self.atom_value(key);
                let found = self.call(trap, handler, &[target, key])?;
                if !found.is_object() {
                    return Ok(found);
                }
                self.complete_descriptor(found)
            }
            None => self.descriptor_checked(target.handle(), key),
        }
    }

    /// A descriptor with everything in it that one should have, whatever it was given with.
    fn complete_descriptor(&mut self, given: Value) -> Run<Value> {
        let out = self.new_plain();
        let handle = out.handle();
        let accessor = self.has_property_checked(given.handle(), crate::text::GET)?
            || self.has_property_checked(given.handle(), crate::text::SET)?;
        if accessor {
            let get = self.get(given, crate::text::GET)?;
            let set = self.get(given, crate::text::SET)?;
            self.define(handle, crate::text::GET, get, PLAIN);
            self.define(handle, crate::text::SET, set, PLAIN);
        } else {
            let value = self.get(given, crate::text::VALUE)?;
            let writable = self.get(given, crate::text::WRITABLE)?;
            self.define(handle, crate::text::VALUE, value, PLAIN);
            let writable = Value::bool(self.to_boolean(writable));
            self.define(handle, crate::text::WRITABLE, writable, PLAIN);
        }
        for name in [crate::text::ENUMERABLE, crate::text::CONFIGURABLE] {
            let value = self.get(given, name)?;
            let value = Value::bool(self.to_boolean(value));
            self.define(handle, name, value, PLAIN);
        }
        Ok(out)
    }

    pub(crate) fn proxy_define(&mut self, object: u32, key: Atom, descriptor: Value) -> Run<()> {
        if let Some(target) = self.namespace_target(Value::object(object)) {
            return self.namespace_define(target, key, descriptor);
        }
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        match self.trap(handler, text::TRAP_DEFINE_PROPERTY)? {
            Some(trap) => {
                let key_value = self.atom_value(key);
                let result = self.call(trap, handler, &[target, key_value, descriptor])?;
                if !self.to_boolean(result) {
                    let name = self.atom_text(key);
                    return Err(self.type_error(&format!(
                        "'defineProperty' on proxy: trap returned falsish for property '{name}'"
                    )));
                }
                Ok(())
            }
            None => self.define_from_descriptor(target.handle(), key, descriptor),
        }
    }

    pub(crate) fn proxy_call(&mut self, object: u32, this: Value, args: &[Value]) -> Run<Value> {
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        match self.trap(handler, text::TRAP_APPLY)? {
            Some(trap) => {
                let list = self.new_array(args.to_vec());
                self.call(trap, handler, &[target, this, list])
            }
            None => self.call(target, this, args),
        }
    }

    pub(crate) fn proxy_construct(
        &mut self,
        object: u32,
        args: &[Value],
        new_target: Value,
    ) -> Run<Value> {
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        match self.trap(handler, text::TRAP_CONSTRUCT)? {
            Some(trap) => {
                let list = self.new_array(args.to_vec());
                let result = self.call(trap, handler, &[target, list, new_target])?;
                if !result.is_object() {
                    return Err(self.type_error("A proxy's construct must give an object"));
                }
                Ok(result)
            }
            None => {
                let new_target = if new_target == Value::object(object) {
                    target
                } else {
                    new_target
                };
                self.construct_with_target(target, args, new_target)
            }
        }
    }

    pub(crate) fn proxy_get_prototype(&mut self, object: u32) -> Run<Value> {
        if self.namespace_target(Value::object(object)).is_some() {
            return Ok(Value::NULL);
        }
        let (target, handler) = self.proxy_of(object).expect("only proxies come here");
        match self.trap(handler, text::TRAP_GET_PROTOTYPE_OF)? {
            Some(trap) => self.call(trap, handler, &[target]),
            None => self.prototype_checked(target),
        }
    }

    /// `Object.getPrototypeOf`, with a proxy's handler having its say.
    pub(crate) fn prototype_checked(&mut self, object: Value) -> Run<Value> {
        if self.proxy_of(object.handle()).is_some() {
            return self.proxy_get_prototype(object.handle());
        }
        let proto = self.proto_of(object.handle());
        Ok(if proto == NONE {
            Value::NULL
        } else {
            Value::object(proto)
        })
    }

    /// Whether this is an array, or a proxy for one.
    pub(crate) fn is_array_value(&self, value: Value) -> bool {
        if !value.is_object() {
            return false;
        }
        match &self.objects[value.index()].kind {
            Kind::Array(_) => true,
            Kind::Proxy(data) => self.is_array_value(data.target),
            _ => false,
        }
    }
}


// A module's namespace, which is what `import * as` gives: a proxy that answers for itself rather than asking a handler.
// Its target has a getter for each name, reading the variable that name is, and the namespace reads those cells
// directly, so it behaves as the language's own kind of object does: what's there is each variable as it is now, which
// can't be written, deleted or added to from outside, and each looks like a value that could be written.
impl Vm {
    /// A namespace over `target`, which holds a getter for each name, or a value for what was copied in from CommonJS.
    pub(crate) fn new_namespace(&mut self, target: Value) -> Value {
        let root = self.shapes.root(NONE);
        let shape = self.shapes.to_dict(root);
        let made = self.alloc_object(
            shape,
            Kind::Proxy(Box::new(ProxyData {
                target,
                handler: Value::UNDEFINED,
                callable: false,
                namespace: true,
            })),
            0,
        );
        self.objects[made.index()].flags |= NOT_EXTENSIBLE;
        made
    }

    /// What a namespace has its names in, if this is one.
    pub(crate) fn namespace_target(&self, value: Value) -> Option<Value> {
        if !value.is_object() {
            return None;
        }
        match &self.objects[value.index()].kind {
            Kind::Proxy(data) if data.namespace => Some(data.target),
            _ => None,
        }
    }

    /// What a name in a namespace has now, or `None` if it isn't one of its names. A variable that hasn't been given
    /// its value yet can't be read, as in the module itself.
    fn namespace_value(&mut self, target: Value, key: Atom) -> Run<Option<Value>> {
        let shape = self.objects[target.index()].shape;
        let Some((slot, flags)) = self.shapes.lookup(shape, key) else {
            return Ok(None);
        };
        let held = self.objects[target.index()].slots()[slot as usize];
        if flags & ACCESSOR == 0 {
            return Ok(Some(held));
        }
        let Kind::Accessor { get, .. } = &self.objects[held.index()].kind else {
            return Ok(None);
        };
        let cell = match &self.objects[get.index()].kind {
            Kind::Native(native) => native.data,
            _ => return Ok(None),
        };
        let value = self.cells[cell.index()];
        if value.is_empty() {
            return Err(self.reference_error("Cannot access a variable before initialization"));
        }
        Ok(Some(value))
    }

    /// Whether a namespace has no names at all.
    pub(crate) fn namespace_is_empty(&mut self, target: Value) -> bool {
        self.own_keys(target.handle()).is_empty()
    }

    fn namespace_get(&mut self, target: Value, key: Atom) -> Run<Value> {
        if key == crate::text::SYMBOL_TO_STRING_TAG {
            return Ok(self.new_str("Module"));
        }
        Ok(self.namespace_value(target, key)?.unwrap_or(Value::UNDEFINED))
    }

    fn namespace_has(&mut self, target: Value, key: Atom) -> bool {
        if key == crate::text::SYMBOL_TO_STRING_TAG {
            return true;
        }
        let shape = self.objects[target.index()].shape;
        self.shapes.lookup(shape, key).is_some()
    }

    fn namespace_refuses(&mut self, doing: &str, key: Atom) -> Value {
        // Code that isn't strict carries on when a write or a delete is refused, as it does for any object.
        self.write_refused = doing != "define";
        let name = self.atom_text(key);
        self.type_error(&match doing {
            "set" => format!("Cannot assign to read only property '{name}' of object '[object Module]'"),
            "delete" => format!("Cannot delete property '{name}' of [object Module]"),
            _ => format!("Cannot redefine property: {name}"),
        })
    }

    /// Its names in the order of their code units, and then `Symbol.toStringTag`.
    fn namespace_keys(&mut self, target: Value) -> Vec<(Atom, u8)> {
        let mut names: Vec<(Vec<u16>, Atom)> = self
            .own_keys(target.handle())
            .into_iter()
            .filter(|(key, _)| self.atoms.kind_of(*key) == crate::text::AtomKind::Name)
            .map(|(key, _)| (self.atom_text(key).encode_utf16().collect(), key))
            .collect();
        names.sort();
        let mut keys: Vec<(Atom, u8)> = names
            .into_iter()
            .map(|(_, key)| (key, ENUMERABLE | WRITABLE))
            .collect();
        keys.push((crate::text::SYMBOL_TO_STRING_TAG, 0));
        keys
    }

    fn namespace_descriptor(&mut self, target: Value, key: Atom) -> Run<Value> {
        let (value, open) = if key == crate::text::SYMBOL_TO_STRING_TAG {
            (self.new_str("Module"), false)
        } else {
            match self.namespace_value(target, key)? {
                Some(value) => (value, true),
                None => return Ok(Value::UNDEFINED),
            }
        };
        let out = self.new_plain();
        let handle = out.handle();
        self.define(handle, crate::text::VALUE, value, PLAIN);
        self.define(handle, crate::text::WRITABLE, Value::bool(open), PLAIN);
        self.define(handle, crate::text::ENUMERABLE, Value::bool(open), PLAIN);
        self.define(handle, crate::text::CONFIGURABLE, Value::FALSE, PLAIN);
        Ok(out)
    }

    /// Defining a name is only allowed to say what it already is.
    fn namespace_define(&mut self, target: Value, key: Atom, descriptor: Value) -> Run<()> {
        let current = self.namespace_descriptor(target, key)?;
        if !current.is_object() {
            return Err(self.namespace_refuses("define", key));
        }
        let given = descriptor.handle();
        let mut fits = true;
        for name in [crate::text::GET, crate::text::SET] {
            fits &= !self.has_property_checked(given, name)?;
        }
        for name in [
            crate::text::CONFIGURABLE,
            crate::text::ENUMERABLE,
            crate::text::WRITABLE,
            crate::text::VALUE,
        ] {
            if self.has_property_checked(given, name)? {
                let wanted = self.get(descriptor, name)?;
                let now = self.get(current, name)?;
                fits &= if name == crate::text::VALUE {
                    self.same_value(wanted, now)
                } else {
                    self.to_boolean(wanted) == self.to_boolean(now)
                };
            }
        }
        if fits {
            Ok(())
        } else {
            Err(self.namespace_refuses("define", key))
        }
    }
}

fn make_proxy(vm: &mut Vm, target: Value, handler: Value) -> Run<Value> {
    if !target.is_object() || !handler.is_object() {
        return Err(vm.type_error("Cannot create proxy with a non-object as target or handler"));
    }
    let callable = vm.is_callable(target);
    // A shape of its own, which nothing can remember, so no read or write goes around the handler.
    let proto = vm.proto_of(target.handle());
    let root = vm.shapes.root(proto);
    let shape = vm.shapes.to_dict(root);
    Ok(vm.alloc_object(
        shape,
        Kind::Proxy(Box::new(ProxyData {
            target,
            handler,
            callable,
            namespace: false,
        })),
        0,
    ))
}

fn proxy_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.new_target.is_object() {
        return Err(vm.type_error("Constructor Proxy requires 'new'"));
    }
    let (target, handler) = (vm.arg(&args, 0), vm.arg(&args, 1));
    make_proxy(vm, target, handler)
}

fn revoke(vm: &mut Vm, args: Args) -> Run<Value> {
    let proxy = vm.native_data_of(&args);
    if let Kind::Proxy(data) = &mut vm.objects[proxy.index()].kind {
        data.handler = Value::NULL;
    }
    Ok(Value::UNDEFINED)
}

fn proxy_revocable(vm: &mut Vm, args: Args) -> Run<Value> {
    let (target, handler) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let proxy = make_proxy(vm, target, handler)?;
    let name = vm.intern_str("revoke");
    let function = vm.new_native(name, 0, revoke, proxy, false);
    let result = vm.new_plain();
    let proxy_atom = vm.intern_str("proxy");
    vm.define(result.handle(), proxy_atom, proxy, PLAIN);
    vm.define(result.handle(), name, function, PLAIN);
    Ok(result)
}

pub(super) fn install(vm: &mut Vm) {
    let atom = vm.intern_str("Proxy");
    let constructor = vm.new_native(atom, 2, proxy_constructor, Value::UNDEFINED, true);
    vm.add_property(vm.global, atom, constructor, HIDDEN);
    vm.materialize_if_lazy(constructor.handle());
    vm.method(constructor.handle(), "revocable", 2, proxy_revocable);
}
