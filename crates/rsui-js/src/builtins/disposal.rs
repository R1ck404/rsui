//! Explicit resource management: what `using` and `await using` do when their block is left, `DisposableStack` and
//! `AsyncDisposableStack`, `SuppressedError`, and the `[Symbol.dispose]` iterators have.
//!
//! Compiled code doesn't have instructions of its own for this. A block with a `using` in it is compiled as a `try` with
//! a `finally`, and both call one native, [`disposal`], with what to do first: a declaration adds what it's given to a
//! list the block keeps in a register, and the `finally` disposes of what's on it. `await using` disposes of one thing at
//! a time, and the compiled code waits for what each gives. The stacks written in `helpers.js` keep the same kind of
//! list and call the same native, so they all dispose in the same way.
//!
//! A list starts with what's happened so far ([`STARTED`] and the rest, then the error to throw at the end) and then has
//! three places for each resource: its value, the method that disposes of it, and how ([`AWAITED`], [`WRAPPED`]). What's
//! being disposed of stays on the list while its method runs, which keeps it from being collected.

use crate::object::*;
use crate::text;
use crate::value::Value;
use crate::vm::{Args, Run, Value0, Vm};

/// `using x = value`: checks it can be disposed of, puts it on the list, and gives it back.
pub(crate) const ADD: i32 = 0;
/// `await using x = value`, the same.
pub(crate) const ADD_AWAITED: i32 = 1;
/// Disposes of everything on the list, last first, given how the block was left: throws what went wrong, if anything did.
pub(crate) const DISPOSE: i32 = 2;
/// Disposes of things on the list until there's something to wait for, and gives that, or gives the list itself once
/// it's done. Throws then, if anything went wrong.
pub(crate) const STEP: i32 = 3;
/// What was waited for after a step failed with this.
pub(crate) const FAILED: i32 = 4;
/// A method of the stacks' own to call, with no value, as `defer` adds.
const ADD_METHOD: i32 = 5;
/// A new `DisposableStack`, or `AsyncDisposableStack`, for `move`, which is always one of the language's own.
const NEW_STACK: i32 = 6;

/// The list has been told how its block was left.
const STARTED: i32 = 1;
/// Something's gone wrong, which is in the list's second place.
const HAS_ERROR: i32 = 2;
/// An `await using` was given `null` or `undefined`, which is still waited for once if nothing else is.
const NEEDS_AWAIT: i32 = 4;
const HAS_AWAITED: i32 = 8;

/// A resource whose disposing is waited for.
const AWAITED: i32 = 1;
/// One an `await using` disposes of through its `[Symbol.dispose]`, as it had no `[Symbol.asyncDispose]`, which is
/// waited for as if it had given a promise of nothing.
const WRAPPED: i32 = 2;

/// Where a list's header ends and its resources start.
const HEADER: usize = 2;

impl Vm {
    /// The native compiled `using` code calls, which is made the first time something's compiled that needs it.
    pub(crate) fn disposal_function(&mut self) -> Value {
        if self.realm.disposal.0.is_undefined() {
            let atom = self.intern_str("disposal");
            let function = self.new_native(atom, 4, disposal, Value::UNDEFINED, false);
            self.realm.disposal = Value0(function);
        }
        self.realm.disposal.0
    }
}

pub(super) fn install(vm: &mut Vm) {
    let error_proto = vm.realm.error_proto;
    let proto = vm.proto_with_parent(error_proto);
    vm.realm.suppressed_error_proto = proto;
    let constructor = vm.constructor("SuppressedError", 3, suppressed_error, proto);
    if let Some(error) = vm.peek_property(error_proto, text::CONSTRUCTOR) {
        vm.set_proto(constructor, error.handle());
    }
    let name = vm.new_str("SuppressedError");
    vm.add_property(proto, text::NAME, name, HIDDEN);
    let message = vm.new_str("");
    vm.add_property(proto, text::MESSAGE, message, HIDDEN);

    let iterator_proto = vm.realm.iterator_proto;
    vm.symbol_method(
        iterator_proto,
        text::SYMBOL_DISPOSE,
        "[Symbol.dispose]",
        0,
        iterator_dispose,
    );
    let async_iterator_proto = vm.proto_of(vm.realm.async_generator_proto);
    let atom = vm.intern_str("[Symbol.asyncDispose]");
    let helper = vm.intern_str("asyncIteratorDispose");
    let key = vm.atom_value(helper);
    let function = vm.new_native(atom, 0, super::call_js_helper, key, false);
    vm.add_property(
        async_iterator_proto,
        text::SYMBOL_ASYNC_DISPOSE,
        function,
        HIDDEN,
    );

    for is_async in [false, true] {
        let proto = vm.plain_proto();
        let (name, func, prefix): (_, NativeFn, _) = if is_async {
            vm.realm.async_disposable_stack_proto = proto;
            ("AsyncDisposableStack", async_disposable_stack, "asyncStack")
        } else {
            vm.realm.disposable_stack_proto = proto;
            ("DisposableStack", disposable_stack, "stack")
        };
        vm.constructor(name, 0, func, proto);
        for (method, length) in [("use", 1), ("adopt", 2), ("defer", 1), ("move", 0)] {
            let helper = format!("{prefix}{}{}", method[..1].to_uppercase(), &method[1..]);
            vm.js_helper(proto, method, &helper, length);
        }
        let (dispose, symbol) = if is_async {
            ("disposeAsync", text::SYMBOL_ASYNC_DISPOSE)
        } else {
            ("dispose", text::SYMBOL_DISPOSE)
        };
        vm.js_helper(proto, dispose, &format!("{prefix}Dispose"), 0);
        // `[Symbol.dispose]` is the same function as `dispose`, and `[Symbol.asyncDispose]` as `disposeAsync`.
        let atom = vm.intern_str(dispose);
        if let Some(function) = vm.peek_property(proto, atom) {
            vm.add_property(proto, symbol, function, HIDDEN);
        }
        let getter_name = vm.intern_str("get disposed");
        let helper = vm.intern_str(&format!("{prefix}Disposed"));
        let key = vm.atom_value(helper);
        let getter = vm.new_native(getter_name, 0, super::call_js_helper, key, false);
        let disposed = vm.intern_str("disposed");
        vm.define_accessor(proto, disposed, Some(getter), None, CONFIGURABLE);
        let tag = vm.new_str(name);
        vm.add_property(proto, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);
    }
}

fn disposable_stack(vm: &mut Vm, args: Args) -> Run<Value> {
    new_stack(vm, &args, false)
}

fn async_disposable_stack(vm: &mut Vm, args: Args) -> Run<Value> {
    new_stack(vm, &args, true)
}

fn new_stack(vm: &mut Vm, args: &Args, is_async: bool) -> Run<Value> {
    let (name, default) = if is_async {
        ("AsyncDisposableStack", vm.realm.async_disposable_stack_proto)
    } else {
        ("DisposableStack", vm.realm.disposable_stack_proto)
    };
    if args.new_target.is_undefined() {
        return Err(vm.type_error(&format!("Constructor {name} requires 'new'")));
    }
    let proto = vm.proto_for(args, default)?;
    let stack = vm.new_object_with_proto(proto, Kind::Plain);
    // What it has to dispose of is kept by `helpers.js`, which has its methods.
    let helpers = vm.load_helpers()?;
    let set_up = vm.get_named(helpers, "stackSetUp")?;
    vm.call(set_up, stack, &[Value::bool(is_async)])?;
    Ok(stack)
}

/// `new SuppressedError(error, suppressed, message)`, which `using` makes when disposing of something fails after
/// something else has.
fn suppressed_error(vm: &mut Vm, args: Args) -> Run<Value> {
    let proto = vm.proto_for(&args, vm.realm.suppressed_error_proto)?;
    let made = vm.new_object_with_proto(proto, Kind::Error);
    let message = vm.arg(&args, 2);
    if !message.is_undefined() {
        let message = vm.to_js_string(message)?;
        vm.define(made.handle(), text::MESSAGE, message, HIDDEN);
    }
    let error = vm.arg(&args, 0);
    let suppressed = vm.arg(&args, 1);
    set_suppressed(vm, made, error, suppressed);
    super::attach_full_stack(vm, made)?;
    Ok(made)
}

fn set_suppressed(vm: &mut Vm, made: Value, error: Value, suppressed: Value) {
    let error_atom = vm.intern_str("error");
    vm.define(made.handle(), error_atom, error, HIDDEN);
    let suppressed_atom = vm.intern_str("suppressed");
    vm.define(made.handle(), suppressed_atom, suppressed, HIDDEN);
}

/// `Iterator.prototype[Symbol.dispose]()`, which calls `return`, as leaving a `for of` early would.
fn iterator_dispose(vm: &mut Vm, args: Args) -> Run<Value> {
    let method = vm.get(args.this, text::RETURN)?;
    if method.is_nullish() {
        return Ok(Value::UNDEFINED);
    }
    if !vm.is_callable(method) {
        return Err(vm.type_error("An iterator's return isn't a function"));
    }
    vm.call(method, args.this, &[])?;
    Ok(Value::UNDEFINED)
}

/// What compiled `using` code and the stacks in `helpers.js` call: what it does is the first argument, one of the
/// numbers above, and the list it does it to is the second.
fn disposal(vm: &mut Vm, args: Args) -> Run<Value> {
    let mode = vm.arg(&args, 0).as_int();
    let list = vm.arg(&args, 1);
    let first = vm.arg(&args, 2);
    let second = vm.arg(&args, 3);
    if mode == NEW_STACK {
        let proto = if first.is_bool() && first == Value::TRUE {
            vm.realm.async_disposable_stack_proto
        } else {
            vm.realm.disposable_stack_proto
        };
        return Ok(vm.new_object_with_proto(proto, Kind::Plain));
    }
    if !list.is_object() || !matches!(vm.objects[list.index()].kind, Kind::Array(_)) {
        return Err(vm.type_error("disposal is only for the engine's own use"));
    }
    match mode {
        ADD | ADD_AWAITED => add(vm, list, first, mode == ADD_AWAITED),
        DISPOSE => dispose(vm, list, first, second),
        STEP => step(vm, list, first, second),
        FAILED => {
            note_error(vm, list, first);
            Ok(Value::UNDEFINED)
        }
        ADD_METHOD => {
            let flags = if second == Value::TRUE { AWAITED } else { 0 };
            push_resource(vm, list, Value::UNDEFINED, first, flags);
            Ok(Value::UNDEFINED)
        }
        _ => Err(vm.type_error("disposal was asked for something it doesn't do")),
    }
}

fn items(vm: &mut Vm, list: Value) -> &mut Items {
    match &mut vm.objects[list.index()].kind {
        Kind::Array(items) => items,
        _ => unreachable!("disposal checked it's given an array"),
    }
}

fn state_of(vm: &mut Vm, list: Value) -> i32 {
    items(vm, list)[0].as_int()
}

fn set_state(vm: &mut Vm, list: Value, state: i32) {
    items(vm, list)[0] = Value::int(state);
}

fn push_resource(vm: &mut Vm, list: Value, value: Value, method: Value, flags: i32) {
    let items = items(vm, list);
    if items.is_empty() {
        items.push(Value::int(0));
        items.push(Value::UNDEFINED);
    }
    items.push(value);
    items.push(method);
    items.push(Value::int(flags));
}

/// A method a value has under `key`, or `undefined` if it has none.
fn method_of(vm: &mut Vm, value: Value, key: text::Atom, name: &str) -> Run<Value> {
    let method = vm.get(value, key)?;
    if method.is_nullish() {
        return Ok(Value::UNDEFINED);
    }
    if !vm.is_callable(method) {
        return Err(vm.type_error(&format!("Symbol({name}) is not a function")));
    }
    Ok(method)
}

fn add(vm: &mut Vm, list: Value, value: Value, awaited: bool) -> Run<Value> {
    if value.is_nullish() {
        // There's nothing to dispose of, but an `await using` still waits.
        if awaited {
            push_resource(vm, list, Value::UNDEFINED, Value::UNDEFINED, AWAITED);
        }
        return Ok(value);
    }
    if !value.is_object() {
        return Err(vm.type_error("An object is expected with `using` declarations"));
    }
    let (method, flags, name) = if awaited {
        let method = method_of(vm, value, text::SYMBOL_ASYNC_DISPOSE, "Symbol.asyncDispose")?;
        if method.is_undefined() {
            let method = method_of(vm, value, text::SYMBOL_DISPOSE, "Symbol.dispose")?;
            (method, AWAITED | WRAPPED, "Symbol.asyncDispose")
        } else {
            (method, AWAITED, "")
        }
    } else {
        let method = method_of(vm, value, text::SYMBOL_DISPOSE, "Symbol.dispose")?;
        (method, 0, "Symbol.dispose")
    };
    if method.is_undefined() {
        return Err(vm.type_error(&format!("Symbol({name}) is not a function")));
    }
    push_resource(vm, list, value, method, flags);
    Ok(value)
}

/// Notes how the block was left, the first time the list's disposed of. A generator told to return leaves its blocks
/// through their `finally`s by a throw of its own, which isn't an error.
fn start(vm: &mut Vm, list: Value, kind: Value, value: Value) {
    if state_of(vm, list) & STARTED != 0 {
        return;
    }
    if kind == Value::int(1) && value != Value::GENERATOR_RETURN {
        items(vm, list)[1] = value;
        set_state(vm, list, STARTED | HAS_ERROR);
    } else {
        set_state(vm, list, STARTED);
    }
}

/// Something going wrong while disposing: if something already had, what's thrown in the end says both.
fn note_error(vm: &mut Vm, list: Value, error: Value) {
    if items(vm, list).is_empty() {
        return;
    }
    let state = state_of(vm, list);
    let error = if state & HAS_ERROR != 0 {
        let earlier = items(vm, list)[1];
        let proto = vm.realm.suppressed_error_proto;
        let made = vm.make_error(proto, "An error was suppressed during disposal");
        set_suppressed(vm, made, error, earlier);
        made
    } else {
        error
    };
    items(vm, list)[1] = error;
    set_state(vm, list, state | HAS_ERROR);
}

/// The last resource on the list, which stays there.
fn last_resource(vm: &mut Vm, list: Value) -> Option<(Value, Value, i32)> {
    let items = items(vm, list);
    let count = items.len();
    (count > HEADER).then(|| {
        (
            items[count - 3],
            items[count - 2],
            items[count - 1].as_int(),
        )
    })
}

fn drop_last_resource(vm: &mut Vm, list: Value) {
    let items = items(vm, list);
    for _ in 0..3 {
        items.pop();
    }
}

/// Throws what went wrong while disposing, if anything did, and empties the list.
fn finish(vm: &mut Vm, list: Value) -> Run<()> {
    let state = state_of(vm, list);
    let error = items(vm, list)[1];
    items(vm, list).vec().clear();
    if state & HAS_ERROR != 0 {
        return Err(error);
    }
    Ok(())
}

fn dispose(vm: &mut Vm, list: Value, kind: Value, value: Value) -> Run<Value> {
    if items(vm, list).is_empty() {
        return Ok(Value::UNDEFINED);
    }
    start(vm, list, kind, value);
    while let Some((value, method, _)) = last_resource(vm, list) {
        let result = if method.is_undefined() {
            Ok(Value::UNDEFINED)
        } else {
            vm.call(method, value, &[])
        };
        drop_last_resource(vm, list);
        if let Err(error) = result {
            note_error(vm, list, error);
        }
    }
    finish(vm, list)?;
    Ok(Value::UNDEFINED)
}

fn step(vm: &mut Vm, list: Value, kind: Value, value: Value) -> Run<Value> {
    if items(vm, list).is_empty() {
        return Ok(list);
    }
    start(vm, list, kind, value);
    loop {
        let state = state_of(vm, list);
        let waits_owed = state & NEEDS_AWAIT != 0 && state & HAS_AWAITED == 0;
        let Some((value, method, flags)) = last_resource(vm, list) else {
            if waits_owed {
                set_state(vm, list, state & !NEEDS_AWAIT);
                return Ok(Value::UNDEFINED);
            }
            finish(vm, list)?;
            return Ok(list);
        };
        // What's disposed of without waiting has to wait for an `await using` of nothing before it first.
        if flags & AWAITED == 0 && waits_owed {
            set_state(vm, list, state & !NEEDS_AWAIT);
            return Ok(Value::UNDEFINED);
        }
        if method.is_undefined() {
            drop_last_resource(vm, list);
            set_state(vm, list, state | NEEDS_AWAIT);
            continue;
        }
        let result = vm.call(method, value, &[]);
        drop_last_resource(vm, list);
        let state = state_of(vm, list);
        match result {
            Ok(given) if flags & AWAITED != 0 => {
                set_state(vm, list, state | HAS_AWAITED);
                return Ok(if flags & WRAPPED != 0 {
                    Value::UNDEFINED
                } else {
                    given
                });
            }
            Ok(_) => {}
            // `[Symbol.dispose]` failing under `await using` would have given a promise that's rejected, which is
            // waited for as long as one that isn't, so it's noted now and something else waited for.
            Err(error) if flags & WRAPPED != 0 => {
                note_error(vm, list, error);
                let state = state_of(vm, list);
                set_state(vm, list, state | HAS_AWAITED);
                return Ok(Value::UNDEFINED);
            }
            Err(error) => note_error(vm, list, error),
        }
    }
}
