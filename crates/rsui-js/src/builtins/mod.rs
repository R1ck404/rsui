//! The standard library: everything a program finds already there when it starts.

mod array;
mod bigint;
pub(crate) mod collate;
#[rustfmt::skip]
mod collate_han;
#[rustfmt::skip]
mod collate_tables;
mod collections;
mod date;
pub(crate) mod disposal;
pub(crate) mod host;
mod inspect;
mod json;
pub(crate) mod locale;
mod math;
mod normalize;
#[rustfmt::skip]
mod normalize_tables;
pub(crate) use math::quick_index as math_quick_index;
#[cfg(feature = "jit")]
pub(crate) use math::{
    quick_apply as math_quick_apply, quick_argc as math_quick_argc, quick_name as math_quick_name,
    quick_native as math_quick_native,
};
mod object;
mod promise;
mod proxy;
mod reflect;
mod string;
mod system;
mod typed;

use crate::object::*;
use crate::text::{self, Atom};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

pub(crate) use bigint::{BigData, BigOp};
pub(crate) use inspect::inspect_value;
pub(crate) use proxy::ProxyData;
pub(crate) use typed::{BufferData, TypedData, ViewData};
#[cfg(feature = "jit")]
pub(crate) use typed::Element;

impl Vm {
    pub(crate) fn method(&mut self, object: u32, name: &str, length: u8, func: NativeFn) {
        let atom = self.intern_str(name);
        let function = self.new_native(atom, length, func, Value::UNDEFINED, false);
        self.add_property(object, atom, function, HIDDEN);
    }

    pub(crate) fn symbol_method(
        &mut self,
        object: u32,
        symbol: Atom,
        name: &str,
        length: u8,
        func: NativeFn,
    ) -> Value {
        let atom = self.intern_str(name);
        let function = self.new_native(atom, length, func, Value::UNDEFINED, false);
        self.add_property(object, symbol, function, HIDDEN);
        function
    }

    pub(crate) fn getter(&mut self, object: u32, name: &str, func: NativeFn) {
        let atom = self.intern_str(name);
        // Its own name says it's a getter, as `get size`.
        let own_name = self.intern_str(&format!("get {name}"));
        let function = self.new_native(own_name, 0, func, Value::UNDEFINED, false);
        self.define_accessor(object, atom, Some(function), None, CONFIGURABLE);
    }

    /// Makes a global constructor with this prototype.
    pub(crate) fn constructor(
        &mut self,
        name: &str,
        length: u8,
        func: NativeFn,
        prototype: u32,
    ) -> u32 {
        let atom = self.intern_str(name);
        let function = self.new_native(atom, length, func, Value::UNDEFINED, true);
        self.materialize_if_lazy(function.handle());
        self.add_property(
            function.handle(),
            text::PROTOTYPE,
            Value::object(prototype),
            0,
        );
        self.add_property(prototype, text::CONSTRUCTOR, function, HIDDEN);
        self.add_property(self.global, atom, function, HIDDEN);
        function.handle()
    }

    /// Puts on an object a method that's written in JavaScript. It's found by name when it's first called.
    pub(crate) fn js_helper(&mut self, object: u32, name: &str, helper: &str, length: u8) {
        let atom = self.intern_str(name);
        let helper = self.intern_str(helper);
        let key = self.atom_value(helper);
        let function = self.new_native(atom, length, call_js_helper, key, false);
        self.add_property(object, atom, function, HIDDEN);
    }

    /// `get [Symbol.species]`, which just gives what it's called on.
    pub(crate) fn species_getter(&mut self, object: u32) {
        fn this_value(_: &mut Vm, args: Args) -> Run<Value> {
            Ok(args.this)
        }
        let atom = self.intern_str("get [Symbol.species]");
        let function = self.new_native(atom, 0, this_value, Value::UNDEFINED, false);
        self.define_accessor(
            object,
            text::SYMBOL_SPECIES,
            Some(function),
            None,
            CONFIGURABLE,
        );
    }

    fn load_helpers(&mut self) -> Run<Value> {
        if !self.helpers.is_undefined() {
            return Ok(self.helpers);
        }
        let file = self.intern_str("rsui:helpers");
        // It's the body of a function that's given the iterator prototype, and what does the work of `using`.
        let proto = self.compile_wrapped(
            include_str!("../js/helpers.js"),
            &["__iteratorPrototype", "__disposal"],
            file,
            // These are the language's own functions, which say they're built in.
            false,
        )?;
        let function = self.run_proto(proto, &[])?;
        let iterator_proto = Value::object(self.realm.iterator_proto);
        let disposal = self.disposal_function();
        let helpers = self.call(function, Value::UNDEFINED, &[iterator_proto, disposal])?;
        self.helpers = helpers;
        Ok(helpers)
    }

    pub(crate) fn plain_proto(&mut self) -> u32 {
        let shape = self.root_shape(self.realm.object_proto);
        self.alloc_object(shape, Kind::Plain, 0).handle()
    }

    pub(crate) fn proto_with_parent(&mut self, parent: u32) -> u32 {
        let shape = self.root_shape(parent);
        self.alloc_object(shape, Kind::Plain, 0).handle()
    }

    pub(crate) fn constant(&mut self, object: u32, name: &str, value: Value) {
        let atom = self.intern_str(name);
        self.add_property(object, atom, value, 0);
    }

    pub(crate) fn global_function(&mut self, name: &str, length: u8, func: NativeFn) {
        self.method(self.global, name, length, func);
    }

    /// Reads a named property, for built-in code that isn't in a hurry.
    pub(crate) fn get_named(&mut self, target: Value, name: &str) -> Run<Value> {
        let atom = self.intern_str(name);
        self.get(target, atom)
    }

    pub(crate) fn native_data_of(&self, args: &Args) -> Value {
        match &self.objects[args.callee as usize].kind {
            Kind::Native(native) => native.data,
            _ => Value::UNDEFINED,
        }
    }

    /// Sets up everything. The first objects are made by hand, as everything else needs them.
    pub(crate) fn install_builtins(&mut self) {
        let null_root = self.shapes.root(NONE);
        let object_proto = self.alloc_object(null_root, Kind::Plain, 0).handle();
        self.objects[object_proto as usize].flags |= IS_PROTO;
        self.realm.object_proto = object_proto;
        self.realm.object_shape = self.shapes.root(object_proto);
        fn nothing(_: &mut Vm, _: Args) -> Run<Value> {
            Ok(Value::UNDEFINED)
        }
        let function_proto = self
            .alloc_object(
                self.realm.object_shape,
                Kind::Native(Box::new(NativeData {
                    func: nothing,
                    data: Value::UNDEFINED,
                    name: text::EMPTY_NAME,
                    length: 0,
                    is_constructor: false,
                    forwards: 0,
                })),
                0,
            )
            .handle();
        self.objects[function_proto as usize].flags |= IS_PROTO;
        self.realm.function_proto = function_proto;
        self.realm.function_shape = self.shapes.root(function_proto);
        let array_proto = self
            .alloc_object(self.realm.object_shape, Kind::Array(Vec::new().into()), 0)
            .handle();
        self.objects[array_proto as usize].flags |= IS_PROTO;
        self.realm.array_proto = array_proto;
        self.realm.array_shape = self.shapes.root(array_proto);
        let global = self.new_plain();
        self.global = global.handle();
        self.objects[self.global as usize].flags |= NEVER_DICT;
        self.constant(self.global, "undefined", Value::UNDEFINED);
        self.constant(self.global, "NaN", Value::double(f64::NAN));
        self.constant(self.global, "Infinity", Value::double(f64::INFINITY));
        let global_this = self.intern_str("globalThis");
        self.add_property(self.global, global_this, global, HIDDEN);
        let absent = self.alloc_object(null_root, Kind::Plain, 0).handle();
        self.add_property(absent, text::FIELDS, Value::UNDEFINED, HIDDEN);
        self.absent = absent;
        self.roots.push(Value::object(absent));

        self.install_object();
        self.install_function();
        self.install_errors();
        self.install_iterators();
        array::install(self);
        string::install(self);
        math::install(self);
        collections::install(self);
        json::install(self);
        promise::install(self);
        date::install(self);
        reflect::install(self);
        inspect::install(self);
        self.install_regexp();
        bigint::install(self);
        typed::install(self);
        proxy::install(self);
        disposal::install(self);
        host::install(self);
        self.install_modules();
    }

    fn install_regexp(&mut self) {
        let proto = self.plain_proto();
        self.realm.regexp_proto = proto;
        let regexp = self.constructor("RegExp", 2, Vm::regexp_constructor, proto);
        self.species_getter(regexp);
        self.js_helper(regexp, "escape", "regExpEscape", 1);
        string::install_regexp_symbols(self, proto);
        self.method(proto, "exec", 1, Vm::regexp_exec);
        self.method(proto, "test", 1, Vm::regexp_test);
        self.method(proto, "toString", 0, Vm::regexp_to_string);
        self.getter(proto, "source", Vm::regexp_source);
        self.getter(proto, "flags", Vm::regexp_flags);
        self.getter(proto, "global", Vm::regexp_global);
        self.getter(proto, "ignoreCase", Vm::regexp_ignore_case);
        self.getter(proto, "multiline", Vm::regexp_multiline);
        self.getter(proto, "sticky", Vm::regexp_sticky);
        self.getter(proto, "unicode", Vm::regexp_unicode);
        self.getter(proto, "dotAll", Vm::regexp_dot_all);
    }

    fn install_iterators(&mut self) {
        let iterator_proto = self.plain_proto();
        self.realm.iterator_proto = iterator_proto;
        fn return_this(_: &mut Vm, args: Args) -> Run<Value> {
            Ok(args.this)
        }
        self.symbol_method(
            iterator_proto,
            text::SYMBOL_ITERATOR,
            "[Symbol.iterator]",
            0,
            return_this,
        );
        let iterator = self.constructor("Iterator", 0, iterator_constructor, iterator_proto);
        self.js_helper(iterator, "from", "iteratorFrom", 1);
        for (name, length) in [
            ("map", 1),
            ("filter", 1),
            ("take", 1),
            ("drop", 1),
            ("flatMap", 1),
            ("reduce", 1),
            ("toArray", 0),
            ("forEach", 1),
            ("some", 1),
            ("every", 1),
            ("find", 1),
        ] {
            self.js_helper(iterator_proto, name, name, length);
        }
        let tag = self.new_str("Iterator");
        self.add_property(
            iterator_proto,
            text::SYMBOL_TO_STRING_TAG,
            tag,
            WRITABLE | CONFIGURABLE,
        );

        let native = self.proto_with_parent(iterator_proto);
        self.realm.native_iterator_proto = native;
        fn native_next(vm: &mut Vm, args: Args) -> Run<Value> {
            if !args.this.is_object()
                || !matches!(vm.objects[args.this.index()].kind, Kind::Iter(_))
            {
                return Err(vm.type_error("next method called on incompatible receiver"));
            }
            match vm.native_iter_step(args.this.handle())? {
                Some(value) => Ok(vm.iter_result(value, false)),
                None => Ok(vm.iter_result(Value::UNDEFINED, true)),
            }
        }
        self.method(native, "next", 0, native_next);

        let generator = self.proto_with_parent(iterator_proto);
        self.realm.generator_proto = generator;
        self.method(generator, "next", 1, Vm::generator_next);
        self.method(generator, "return", 1, Vm::generator_return);
        self.method(generator, "throw", 1, Vm::generator_throw);

        let tag = self.new_str("Generator");
        self.add_property(generator, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);

        // Each kind of function has its own prototype and constructor, which programs can reach through a function.
        let parent = self.realm.function_proto;
        let generator_function = self.proto_with_parent(parent);
        self.realm.generator_function_proto = generator_function;
        self.realm.generator_function_shape = self.root_shape(generator_function);
        let async_function = self.proto_with_parent(parent);
        self.realm.async_function_proto = async_function;
        self.realm.async_function_shape = self.root_shape(async_function);
        let async_generator_function = self.proto_with_parent(parent);
        self.realm.async_generator_function_proto = async_generator_function;
        self.realm.async_generator_function_shape = self.root_shape(async_generator_function);
        self.add_property(
            generator_function,
            text::PROTOTYPE,
            Value::object(generator),
            CONFIGURABLE,
        );
        self.add_property(
            generator,
            text::CONSTRUCTOR,
            Value::object(generator_function),
            CONFIGURABLE,
        );
        let function_constructor = self
            .get_named(Value::object(parent), "constructor")
            .unwrap_or(Value::UNDEFINED);
        for (name, proto) in [
            ("GeneratorFunction", generator_function),
            ("AsyncFunction", async_function),
            ("AsyncGeneratorFunction", async_generator_function),
        ] {
            let atom = self.intern_str(name);
            let constructor = self.new_native(
                atom,
                1,
                object::function_constructor,
                Value::UNDEFINED,
                true,
            );
            self.materialize_if_lazy(constructor.handle());
            self.add_property(
                constructor.handle(),
                text::PROTOTYPE,
                Value::object(proto),
                0,
            );
            self.add_property(proto, text::CONSTRUCTOR, constructor, CONFIGURABLE);
            let tag = self.new_str(name);
            self.add_property(proto, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);
            if function_constructor.is_object() {
                self.set_proto(constructor.handle(), function_constructor.handle());
            }
        }

        let async_iterator_proto = self.plain_proto();
        self.symbol_method(
            async_iterator_proto,
            text::SYMBOL_ASYNC_ITERATOR,
            "[Symbol.asyncIterator]",
            0,
            return_this,
        );
        let async_generator = self.proto_with_parent(async_iterator_proto);
        self.realm.async_generator_proto = async_generator;
        let tag = self.new_str("AsyncGenerator");
        self.add_property(
            async_generator,
            text::SYMBOL_TO_STRING_TAG,
            tag,
            CONFIGURABLE,
        );
        self.add_property(
            async_generator_function,
            text::PROTOTYPE,
            Value::object(async_generator),
            CONFIGURABLE,
        );
        self.add_property(
            async_generator,
            text::CONSTRUCTOR,
            Value::object(async_generator_function),
            CONFIGURABLE,
        );
        self.method(async_generator, "next", 1, Vm::async_generator_next);
        self.method(async_generator, "return", 1, Vm::async_generator_return);
        self.method(async_generator, "throw", 1, Vm::async_generator_throw);
    }

    fn install_errors(&mut self) {
        let proto = self.plain_proto();
        self.realm.error_proto = proto;
        let constructor = self.constructor("Error", 1, error_constructor, proto);
        let name = self.new_str("Error");
        self.add_property(proto, text::NAME, name, HIDDEN);
        let message = self.new_str("");
        self.add_property(proto, text::MESSAGE, message, HIDDEN);
        self.method(proto, "toString", 0, error_to_string);
        self.method(constructor, "captureStackTrace", 2, capture_stack_trace);
        self.method(constructor, "isError", 1, is_error);
        let thrower = self.intern_str("");
        self.throw_type_error = self.new_native(
            thrower,
            0,
            throw_type_error_function,
            Value::UNDEFINED,
            false,
        );
        for (name, slot) in [
            ("TypeError", 0),
            ("RangeError", 1),
            ("ReferenceError", 2),
            ("SyntaxError", 3),
            ("EvalError", 4),
            ("URIError", 5),
            ("AggregateError", 6),
        ] {
            let sub = self.proto_with_parent(proto);
            let func: NativeFn = if slot == 6 {
                aggregate_error_constructor
            } else {
                error_constructor
            };
            let made = self.constructor(name, 1, func, sub);
            self.set_proto(made, constructor);
            let name_value = self.new_str(name);
            self.add_property(sub, text::NAME, name_value, HIDDEN);
            let empty = self.new_str("");
            self.add_property(sub, text::MESSAGE, empty, HIDDEN);
            match slot {
                0 => self.realm.type_error_proto = sub,
                1 => self.realm.range_error_proto = sub,
                2 => self.realm.reference_error_proto = sub,
                3 => self.realm.syntax_error_proto = sub,
                _ => {}
            }
        }
    }
}

fn call_js_helper(vm: &mut Vm, args: Args) -> Run<Value> {
    let key = vm.native_data_of(&args);
    let helpers = vm.load_helpers()?;
    let atom = vm.to_atom(key)?;
    let function = vm.get(helpers, atom)?;
    let list: Vec<Value> = (0..args.count).map(|index| vm.arg(&args, index)).collect();
    vm.call(function, args.this, &list)
}

fn iterator_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.new_target.is_object() || args.new_target.handle() == args.callee {
        return Err(
            vm.type_error("Iterator can't be used directly, only through a class that extends it")
        );
    }
    let proto = vm.proto_for(&args, vm.realm.iterator_proto)?;
    Ok(vm.new_object_with_proto(proto, Kind::Plain))
}

fn is_error(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(
        value.is_object() && matches!(vm.objects[value.index()].kind, Kind::Error),
    ))
}

fn throw_type_error_function(vm: &mut Vm, _: Args) -> Run<Value> {
    Err(vm.type_error(
        "'caller', 'callee', and 'arguments' properties may not be accessed on strict mode functions",
    ))
}

fn error_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let default = match &vm.objects[args.callee as usize].kind {
        Kind::Native(_) => {
            let callee = Value::object(args.callee);
            let proto = vm.get(callee, text::PROTOTYPE)?;
            proto.handle()
        }
        _ => vm.realm.error_proto,
    };
    let proto = vm.proto_for(&args, default)?;
    let error = vm.new_object_with_proto(proto, Kind::Error);
    let message = vm.arg(&args, 0);
    if !message.is_undefined() {
        let message = vm.to_js_string(message)?;
        vm.define(error.handle(), text::MESSAGE, message, HIDDEN);
    }
    let options = vm.arg(&args, 1);
    if options.is_object() && vm.has_property(options.handle(), text::CAUSE) {
        let cause = vm.get(options, text::CAUSE)?;
        vm.define(error.handle(), text::CAUSE, cause, HIDDEN);
    }
    attach_full_stack(vm, error)?;
    Ok(error)
}

fn aggregate_error_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let default = {
        let callee = Value::object(args.callee);
        vm.get(callee, text::PROTOTYPE)?.handle()
    };
    let proto = vm.proto_for(&args, default)?;
    let error = vm.new_object_with_proto(proto, Kind::Error);
    let list = vm.arg(&args, 0);
    let items = vm.iterate_to_vec(list)?;
    let array = vm.new_array(items);
    vm.define(error.handle(), text::ERRORS, array, HIDDEN);
    let message = vm.arg(&args, 1);
    if !message.is_undefined() {
        let message = vm.to_js_string(message)?;
        vm.define(error.handle(), text::MESSAGE, message, HIDDEN);
    }
    attach_full_stack(vm, error)?;
    Ok(error)
}

/// `stack` says what the error is first, and then where it was made.
pub(crate) fn attach_full_stack(vm: &mut Vm, error: Value) -> Run<()> {
    let heading = quiet_heading(vm, error);
    let trace = vm.stack_trace();
    let full = if trace.is_empty() {
        heading
    } else {
        format!("{heading}\n{trace}")
    };
    let full = vm.new_str(&full);
    vm.define(error.handle(), text::STACK, full, HIDDEN);
    Ok(())
}

/// What an error says it is, for its `stack`. This is done while it's being made, when a subclass hasn't set up what its own `name` or
/// `message` might use, so nothing of the program's runs for this. If it has a getter for those, this says what class it is instead.
fn quiet_heading(vm: &mut Vm, error: Value) -> String {
    let object = error.handle();
    let text_of =
        |vm: &mut Vm, value: Value| value.is_string().then(|| vm.text_of(value).to_rust());
    let name = match vm.peek_property(object, text::NAME) {
        Some(value) => text_of(vm, value),
        None => vm
            .peek_property(object, text::CONSTRUCTOR)
            .filter(|found| found.is_object())
            .and_then(|found| vm.peek_property(found.handle(), text::NAME))
            .and_then(|value| text_of(vm, value)),
    }
    .unwrap_or_else(|| "Error".to_owned());
    let message = vm
        .peek_property(object, text::MESSAGE)
        .and_then(|value| text_of(vm, value))
        .unwrap_or_default();
    if message.is_empty() {
        name
    } else if name.is_empty() {
        message
    } else {
        format!("{name}: {message}")
    }
}

fn error_heading(vm: &mut Vm, error: Value) -> Run<String> {
    let name = vm.get(error, text::NAME)?;
    let name = if name.is_undefined() {
        "Error".to_owned()
    } else {
        vm.to_rust_string(name)?
    };
    let message = vm.get(error, text::MESSAGE)?;
    let message = if message.is_undefined() {
        String::new()
    } else {
        vm.to_rust_string(message)?
    };
    Ok(if message.is_empty() {
        name
    } else if name.is_empty() {
        message
    } else {
        format!("{name}: {message}")
    })
}

fn error_to_string(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.this.is_object() {
        return Err(vm.type_error("Error.prototype.toString requires that 'this' be an Object"));
    }
    let heading = error_heading(vm, args.this)?;
    Ok(vm.new_str(&heading))
}

fn capture_stack_trace(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = vm.arg(&args, 0);
    if target.is_object() {
        attach_full_stack(vm, target)?;
    }
    Ok(Value::UNDEFINED)
}
