//! `Promise`, `queueMicrotask` and timers.

use crate::object::*;
use crate::text;
use crate::value::Value;
use crate::vm::{Args, Job, Run, Timer, Vm};

/// Which of the things that [`Vm::constructor_of`] remembers is for promises.
const PROMISE_MEMO: usize = 1;

pub(super) fn install(vm: &mut Vm) {
    let proto = vm.plain_proto();
    vm.realm.promise_proto = proto;
    let promise = vm.constructor("Promise", 1, promise_constructor, proto);
    vm.realm.promise_constructor = promise;
    vm.method(proto, "then", 2, then);
    vm.method(proto, "catch", 1, catch);
    vm.method(proto, "finally", 1, finally);
    let tag = vm.new_str("Promise");
    vm.add_property(proto, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);
    vm.method(promise, "resolve", 1, static_resolve);
    vm.method(promise, "reject", 1, static_reject);
    vm.method(promise, "all", 1, all);
    vm.method(promise, "allSettled", 1, all_settled);
    vm.method(promise, "any", 1, any);
    vm.method(promise, "race", 1, race);
    vm.method(promise, "withResolvers", 0, with_resolvers);
    vm.js_helper(promise, "try", "promiseTry", 1);
    vm.species_getter(promise);

    vm.global_function("queueMicrotask", 1, queue_microtask);
    vm.global_function("setTimeout", 2, set_timeout);
    vm.global_function("setInterval", 2, set_interval);
    vm.global_function("setImmediate", 1, set_immediate);
    vm.global_function("clearTimeout", 1, clear_timer);
    vm.global_function("clearInterval", 1, clear_timer);
    vm.global_function("clearImmediate", 1, clear_timer);
}

impl Vm {
    pub(crate) fn new_promise(&mut self) -> Value {
        let proto = self.realm.promise_proto;
        self.new_object_with_proto(
            proto,
            Kind::Promise(Box::new(PromiseData {
                state: PromiseState::Pending,
                value: Value::UNDEFINED,
                reactions: Vec::new(),
                handled: false,
            })),
        )
    }

    /// Makes a promise with what's given as a constructor, along with what settles it. That's a promise of this engine's
    /// own at once if it's `Promise`, and otherwise whatever the constructor makes of being handed an executor.
    pub(crate) fn new_capability(&mut self, constructor: Value) -> Run<(Value, Value, Value)> {
        if constructor == Value::object(self.realm.promise_constructor) {
            let promise = self.new_promise();
            let (resolve, reject) = self.resolving_functions(promise);
            return Ok((promise, resolve, reject));
        }
        if !self.is_constructor(constructor) {
            return Err(self.type_error("Promise capability needs a constructor"));
        }
        let held = self.new_array(vec![Value::UNDEFINED, Value::UNDEFINED]);
        let executor = self.new_native(text::EMPTY_NAME, 2, capability_executor, held, false);
        let promise = self.construct(constructor, &[executor])?;
        let items = self.array_items(held);
        if !self.is_callable(items[0]) || !self.is_callable(items[1]) {
            return Err(self.type_error("Promise resolve or reject function is not callable"));
        }
        Ok((promise, items[0], items[1]))
    }

    /// `Promise.resolve`, for this constructor: what it already made stays as it is.
    fn promise_resolve_with(&mut self, constructor: Value, value: Value) -> Run<Value> {
        if constructor == Value::object(self.realm.promise_constructor) {
            return self.promise_resolve_value(value);
        }
        if self.is_promise(value) {
            let made_by = self.get(value, text::CONSTRUCTOR)?;
            if made_by == constructor {
                return Ok(value);
            }
        }
        let (promise, resolve, _) = self.new_capability(constructor)?;
        self.call(resolve, Value::UNDEFINED, &[value])?;
        Ok(promise)
    }

    /// Settles what a `then` made, which is either a promise of this engine's own, or what settles another kind of one.
    fn settle_derived(&mut self, derived: Value, is_reject: bool, value: Value) {
        if self.is_promise(derived) {
            if is_reject {
                self.reject_promise(derived, value);
            } else {
                self.resolve_promise(derived, value);
            }
        } else if derived.is_object()
            && matches!(self.objects[derived.index()].kind, Kind::Array(_))
        {
            let settle = self.array_items(derived)[usize::from(is_reject)];
            // There's nobody to tell if this goes wrong.
            let _ = self.call(settle, Value::UNDEFINED, &[value]);
        }
    }

    fn is_promise(&self, value: Value) -> bool {
        value.is_object() && matches!(self.objects[value.index()].kind, Kind::Promise(_))
    }

    fn settle(&mut self, promise: Value, state: PromiseState, value: Value) {
        let Kind::Promise(data) = &mut self.objects[promise.index()].kind else {
            return;
        };
        if data.state != PromiseState::Pending {
            return;
        }
        data.state = state;
        data.value = value;
        let reactions = std::mem::take(&mut data.reactions);
        let handled = data.handled;
        if state == PromiseState::Rejected && !handled {
            self.unhandled_rejections.push(promise);
        }
        for reaction in reactions {
            self.queue_reaction(&reaction, state, value);
        }
    }

    fn queue_reaction(&mut self, reaction: &Reaction, state: PromiseState, value: Value) {
        let is_reject = state == PromiseState::Rejected;
        self.jobs.push_back(Job::Reaction {
            handler: if is_reject {
                reaction.on_rejected
            } else {
                reaction.on_fulfilled
            },
            argument: value,
            derived: reaction.derived,
            is_reject,
        });
    }

    pub(crate) fn reject_promise(&mut self, promise: Value, reason: Value) {
        self.settle(promise, PromiseState::Rejected, reason);
    }

    /// Settles a promise with a value, which if it has a `then` means waiting for that.
    pub(crate) fn resolve_promise(&mut self, promise: Value, value: Value) {
        if value == promise {
            let error = self.type_error("Chaining cycle detected for promise");
            self.reject_promise(promise, error);
            return;
        }
        if !value.is_object() {
            self.settle(promise, PromiseState::Fulfilled, value);
            return;
        }
        let then = match self.get(value, text::THEN) {
            Ok(then) => then,
            Err(error) => {
                self.reject_promise(promise, error);
                return;
            }
        };
        if !self.is_callable(then) {
            self.settle(promise, PromiseState::Fulfilled, value);
            return;
        }
        self.jobs.push_back(Job::Thenable {
            promise,
            thenable: value,
            then,
        });
    }

    /// `Promise.resolve`: a promise stays as it is, and anything else becomes one.
    pub(crate) fn promise_resolve_value(&mut self, value: Value) -> Run<Value> {
        if self.is_promise(value) {
            let made_by = self.constructor_of(value, self.realm.promise_proto, PROMISE_MEMO)?;
            if made_by == Value::object(self.realm.promise_constructor) {
                return Ok(value);
            }
        }
        let promise = self.new_promise();
        self.resolve_promise(promise, value);
        Ok(promise)
    }

    pub(crate) fn promise_then(
        &mut self,
        promise: Value,
        on_fulfilled: Value,
        on_rejected: Value,
        derived: Value,
    ) {
        let Kind::Promise(data) = &mut self.objects[promise.index()].kind else {
            return;
        };
        let state = data.state;
        let value = data.value;
        let was_handled = data.handled;
        data.handled = true;
        let reaction = Reaction {
            on_fulfilled,
            on_rejected,
            derived,
        };
        if state == PromiseState::Pending {
            data.reactions.push(reaction);
            return;
        }
        if state == PromiseState::Rejected && !was_handled {
            self.unhandled_rejections.retain(|found| *found != promise);
        }
        self.queue_reaction(&reaction, state, value);
    }

    /// A pair of functions that settle a promise, once between them.
    fn resolving_functions(&mut self, promise: Value) -> (Value, Value) {
        let guard = self.new_array(vec![promise, Value::FALSE]);
        let resolve = self.new_native(text::EMPTY_NAME, 1, resolve_function, guard, false);
        let reject = self.new_native(text::EMPTY_NAME, 1, reject_function, guard, false);
        (resolve, reject)
    }

    fn run_job(&mut self, job: Job) -> Run<()> {
        match job {
            Job::Reaction {
                handler,
                argument,
                derived,
                is_reject,
            } => {
                // An async function or generator waiting for this promise. See `Vm::resume_waiting`. Nothing else may
                // hold it while it runs.
                if handler.is_object()
                    && matches!(self.objects[handler.index()].kind, Kind::Generator(_))
                {
                    self.nursery.push(handler);
                    self.resume_waiting(handler, is_reject, argument);
                    return Ok(());
                }
                if !self.is_callable(handler) {
                    self.settle_derived(derived, is_reject, argument);
                    return Ok(());
                }
                // Nothing else may hold the promise `then` gave back, and the handler can make the collector run.
                self.nursery.push(derived);
                match self.call(handler, Value::UNDEFINED, &[argument]) {
                    Ok(value) => self.settle_derived(derived, false, value),
                    Err(error) => self.settle_derived(derived, true, error),
                }
                Ok(())
            }
            Job::Thenable {
                promise,
                thenable,
                then,
            } => {
                let (resolve, reject) = self.resolving_functions(promise);
                if let Err(error) = self.call(then, thenable, &[resolve, reject]) {
                    self.call(reject, Value::UNDEFINED, &[error])?;
                }
                Ok(())
            }
            Job::Call(function) => {
                self.call(function, Value::UNDEFINED, &[])?;
                Ok(())
            }
        }
    }

    /// Runs everything that's waiting to run right now. An error nobody can catch stops it, and is given back.
    ///
    /// Once there's nothing left and nothing is running, what `WeakRef`s were keeping alive till then can go, and each
    /// `FinalizationRegistry` whose targets have gone has its callback called, as a job of its own, after which the jobs
    /// that made are run in turn. That's only the registries that were waiting before this started: as in Node, a
    /// registry's callback runs in a later turn than the one its targets went in, after the timers that are due then.
    pub(crate) fn run_jobs(&mut self) -> Run<()> {
        let mut due = if self.frames.is_empty() {
            self.cleanups.len()
        } else {
            0
        };
        loop {
            while let Some(job) = self.jobs.pop_front() {
                self.run_job(job)?;
                // Between jobs nothing is held by native code, unless this is inside a native.
                if self.debt > self.threshold && self.frames.is_empty() {
                    self.collect_garbage();
                }
            }
            if !self.frames.is_empty() {
                return Ok(());
            }
            // The next turn's calls reuse the registers this one's left behind, which a call doesn't clear all of, and
            // what's still in one would then stay alive, as if a `WeakRef` or a `WeakMap` held it strongly.
            self.wipe_used_above_top();
            self.clear_kept_objects();
            if due == 0 {
                return Ok(());
            }
            due -= 1;
            let Some(registry) = self.cleanups.pop_front() else {
                return Ok(());
            };
            self.clean_up_registry(registry)?;
        }
    }

    /// Runs the timers that are due. It says when the next one will be, if there is one.
    pub(crate) fn run_timers(&mut self, now: f64) -> Run<Option<f64>> {
        loop {
            let mut due: Option<usize> = None;
            for (index, timer) in self.timers.iter().enumerate() {
                if timer.at <= now
                    && due.is_none_or(|best| {
                        let other = &self.timers[best];
                        timer.at < other.at || (timer.at == other.at && timer.id < other.id)
                    })
                {
                    due = Some(index);
                }
            }
            let Some(index) = due else {
                break;
            };
            let callback = self.timers[index].callback;
            let args = self.timers[index].args.clone();
            if let Some(every) = self.timers[index].repeat {
                self.timers[index].at = now + every.max(1.0);
            } else {
                let spent = self.timers.swap_remove(index);
                self.keep_spent_timer(&spent);
            }
            self.call(callback, Value::UNDEFINED, &args)?;
            self.run_jobs()?;
        }
        Ok(self
            .timers
            .iter()
            .map(|timer| timer.at)
            .min_by(f64::total_cmp))
    }

    pub(crate) fn now_ms() -> f64 {
        crate::inspect::clock(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |time| time.as_secs_f64() * 1000.0)
        })
    }
}

/// What a constructor is handed, to get hold of what settles the promise it makes.
fn capability_executor(vm: &mut Vm, args: Args) -> Run<Value> {
    let held = vm.native_data_of(&args);
    let items = vm.array_items(held);
    if !items[0].is_undefined() || !items[1].is_undefined() {
        return Err(vm.type_error("Promise executor has already been called"));
    }
    let (resolve, reject) = (vm.arg(&args, 0), vm.arg(&args, 1));
    vm.set_index(held, 0, resolve)?;
    vm.set_index(held, 1, reject)?;
    Ok(Value::UNDEFINED)
}

fn resolve_function(vm: &mut Vm, args: Args) -> Run<Value> {
    let guard = vm.native_data_of(&args);
    let items = vm.array_items(guard);
    if items[1] == Value::TRUE {
        return Ok(Value::UNDEFINED);
    }
    vm.set_index(guard, 1, Value::TRUE)?;
    let value = vm.arg(&args, 0);
    vm.resolve_promise(items[0], value);
    Ok(Value::UNDEFINED)
}

fn reject_function(vm: &mut Vm, args: Args) -> Run<Value> {
    let guard = vm.native_data_of(&args);
    let items = vm.array_items(guard);
    if items[1] == Value::TRUE {
        return Ok(Value::UNDEFINED);
    }
    vm.set_index(guard, 1, Value::TRUE)?;
    let value = vm.arg(&args, 0);
    vm.reject_promise(items[0], value);
    Ok(Value::UNDEFINED)
}

fn promise_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.new_target.is_object() {
        return Err(vm.type_error("Promise constructor cannot be invoked without 'new'"));
    }
    let executor = vm.arg(&args, 0);
    if !vm.is_callable(executor) {
        let shown = vm.brief(executor);
        return Err(vm.type_error(&format!("Promise resolver {shown} is not a function")));
    }
    let proto = vm.proto_for(&args, vm.realm.promise_proto)?;
    let promise = vm.new_promise();
    if proto != vm.realm.promise_proto {
        vm.set_proto(promise.handle(), proto);
    }
    let (resolve, reject) = vm.resolving_functions(promise);
    if let Err(error) = vm.call(executor, Value::UNDEFINED, &[resolve, reject]) {
        vm.call(reject, Value::UNDEFINED, &[error])?;
    }
    Ok(promise)
}

fn this_promise(vm: &mut Vm, this: Value) -> Run<Value> {
    if vm.is_promise(this) {
        return Ok(this);
    }
    Err(vm.type_error("Method Promise.prototype.then called on incompatible receiver"))
}

fn then(vm: &mut Vm, args: Args) -> Run<Value> {
    let promise = this_promise(vm, args.this)?;
    let (on_fulfilled, on_rejected) = (vm.arg(&args, 0), vm.arg(&args, 1));
    let plain = Value::object(vm.realm.promise_constructor);
    let found = vm.constructor_of(promise, vm.realm.promise_proto, PROMISE_MEMO)?;
    let constructor = if found == plain {
        plain
    } else {
        vm.species_from(found, plain)?
    };
    if constructor == plain {
        let derived = vm.new_promise();
        vm.promise_then(promise, on_fulfilled, on_rejected, derived);
        return Ok(derived);
    }
    let (result, resolve, reject) = vm.new_capability(constructor)?;
    let derived = vm.new_array(vec![resolve, reject]);
    vm.promise_then(promise, on_fulfilled, on_rejected, derived);
    Ok(result)
}

fn catch(vm: &mut Vm, args: Args) -> Run<Value> {
    let on_rejected = vm.arg(&args, 0);
    let method = vm.get(args.this, text::THEN)?;
    vm.call(method, args.this, &[Value::UNDEFINED, on_rejected])
}

/// After the callback of a `finally` has run and been waited for, which goes on to `pass`. What each of these is given
/// is the callback, and what's making the promises.
fn after_finally(vm: &mut Vm, args: &Args, pass: NativeFn) -> Run<Value> {
    let items = vm.array_items(vm.native_data_of(args));
    let value = vm.arg(args, 0);
    let result = vm.call(items[0], Value::UNDEFINED, &[])?;
    let waiting = vm.promise_resolve_with(items[1], result)?;
    let pass = vm.new_native(text::EMPTY_NAME, 0, pass, value, false);
    let method = vm.get(waiting, text::THEN)?;
    vm.call(method, waiting, &[pass])
}

fn finally_fulfilled(vm: &mut Vm, args: Args) -> Run<Value> {
    after_finally(vm, &args, return_data)
}

fn finally_rejected(vm: &mut Vm, args: Args) -> Run<Value> {
    after_finally(vm, &args, throw_data)
}

fn return_data(vm: &mut Vm, args: Args) -> Run<Value> {
    Ok(vm.native_data_of(&args))
}

fn throw_data(vm: &mut Vm, args: Args) -> Run<Value> {
    Err(vm.native_data_of(&args))
}

fn finally(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.this.is_object() {
        return Err(vm.type_error("Promise.prototype.finally called on a non-object"));
    }
    let plain = Value::object(vm.realm.promise_constructor);
    let constructor = vm.species_constructor(args.this, plain)?;
    let callback = vm.arg(&args, 0);
    let method = vm.get(args.this, text::THEN)?;
    if !vm.is_callable(callback) {
        return vm.call(method, args.this, &[callback, callback]);
    }
    let data = vm.new_array(vec![callback, constructor]);
    let on_fulfilled = vm.new_native(text::EMPTY_NAME, 1, finally_fulfilled, data, false);
    let on_rejected = vm.new_native(text::EMPTY_NAME, 1, finally_rejected, data, false);
    vm.call(method, args.this, &[on_fulfilled, on_rejected])
}

fn static_resolve(vm: &mut Vm, args: Args) -> Run<Value> {
    if !args.this.is_object() {
        return Err(vm.type_error("Promise.resolve called on a non-object"));
    }
    let value = vm.arg(&args, 0);
    vm.promise_resolve_with(args.this, value)
}

fn static_reject(vm: &mut Vm, args: Args) -> Run<Value> {
    let reason = vm.arg(&args, 0);
    let (promise, _, reject) = vm.new_capability(args.this)?;
    vm.call(reject, Value::UNDEFINED, &[reason])?;
    Ok(promise)
}

fn with_resolvers(vm: &mut Vm, args: Args) -> Run<Value> {
    let (promise, resolve, reject) = vm.new_capability(args.this)?;
    let result = vm.new_plain();
    let promise_atom = vm.intern_str("promise");
    vm.define(result.handle(), promise_atom, promise, PLAIN);
    let resolve_atom = vm.intern_str("resolve");
    vm.define(result.handle(), resolve_atom, resolve, PLAIN);
    let reject_atom = vm.intern_str("reject");
    vm.define(result.handle(), reject_atom, reject, PLAIN);
    Ok(result)
}

// `all`, `allSettled` and `any` all count what's still to come, and collect into an array.
// What each of their helper functions needs is kept in an array: the results, how many are left,
// the promise to settle, this helper's place in the results, and whether it's been called.
const RESULTS: usize = 0;
const REMAINING: usize = 1;
const TARGET: usize = 2;
const PLACE: usize = 3;

#[derive(Clone, Copy, PartialEq)]
enum Combinator {
    All,
    Settled,
    Any,
}

/// Puts a value in the results, which is only ever the array's own place, whatever `Array.prototype` has been given.
fn store_result(vm: &mut Vm, results: Value, place: usize, value: Value) {
    if let Kind::Array(found) = &mut vm.objects[results.index()].kind {
        let mut items = found.vec();
        if items.len() <= place {
            items.resize(place + 1, Value::UNDEFINED);
        }
        items[place] = value;
    }
}

fn finish_one(vm: &mut Vm, state: Value, place: usize, value: Value, mode: Combinator) -> Run<()> {
    let items = vm.array_items(state);
    store_result(vm, items[RESULTS], place, value);
    let remaining = vm.get_index(items[REMAINING], 0)?.as_number() - 1.0;
    vm.set_index(items[REMAINING], 0, Value::number(remaining))?;
    if remaining == 0.0 {
        finish_all(vm, items[TARGET], items[RESULTS], mode)?;
    }
    Ok(())
}

fn finish_all(vm: &mut Vm, target: Value, results: Value, mode: Combinator) -> Run<()> {
    // This is what settles the promise, in that order.
    let settle = vm.array_items(target);
    if mode == Combinator::Any {
        let constructor = vm.get_named(Value::object(vm.global), "AggregateError")?;
        let message = vm.new_str("All promises were rejected");
        let error = vm.construct(constructor, &[results, message])?;
        vm.call(settle[1], Value::UNDEFINED, &[error])?;
    } else {
        vm.call(settle[0], Value::UNDEFINED, &[results])?;
    }
    Ok(())
}

/// Where a helper's value goes, unless this has already been called, as it only counts the first time.
fn take_place(vm: &mut Vm, state: Value) -> Run<Option<usize>> {
    let place = vm.get_index(state, PLACE)?.as_number();
    if place < 0.0 {
        return Ok(None);
    }
    vm.set_index(state, PLACE, Value::int(-1))?;
    Ok(Some(place as usize))
}

fn settled_object(vm: &mut Vm, status: &str, key: &str, value: Value) -> Value {
    let object = vm.new_plain();
    let status_atom = vm.intern_str("status");
    let status_value = vm.new_str(status);
    vm.define(object.handle(), status_atom, status_value, PLAIN);
    let key_atom = vm.intern_str(key);
    vm.define(object.handle(), key_atom, value, PLAIN);
    object
}

fn all_element(vm: &mut Vm, args: Args) -> Run<Value> {
    let state = vm.native_data_of(&args);
    let Some(place) = take_place(vm, state)? else {
        return Ok(Value::UNDEFINED);
    };
    let value = vm.arg(&args, 0);
    finish_one(vm, state, place, value, Combinator::All)?;
    Ok(Value::UNDEFINED)
}

fn settled_fulfilled(vm: &mut Vm, args: Args) -> Run<Value> {
    let state = vm.native_data_of(&args);
    let Some(place) = take_place(vm, state)? else {
        return Ok(Value::UNDEFINED);
    };
    let value = vm.arg(&args, 0);
    let object = settled_object(vm, "fulfilled", "value", value);
    finish_one(vm, state, place, object, Combinator::Settled)?;
    Ok(Value::UNDEFINED)
}

fn settled_rejected(vm: &mut Vm, args: Args) -> Run<Value> {
    let state = vm.native_data_of(&args);
    let Some(place) = take_place(vm, state)? else {
        return Ok(Value::UNDEFINED);
    };
    let value = vm.arg(&args, 0);
    let object = settled_object(vm, "rejected", "reason", value);
    finish_one(vm, state, place, object, Combinator::Settled)?;
    Ok(Value::UNDEFINED)
}

fn any_rejected(vm: &mut Vm, args: Args) -> Run<Value> {
    let state = vm.native_data_of(&args);
    let Some(place) = take_place(vm, state)? else {
        return Ok(Value::UNDEFINED);
    };
    let value = vm.arg(&args, 0);
    finish_one(vm, state, place, value, Combinator::Any)?;
    Ok(Value::UNDEFINED)
}

fn combine(vm: &mut Vm, args: &Args, mode: Combinator) -> Run<Value> {
    let source = vm.arg(args, 0);
    let this = args.this;
    let (promise_made, resolve_target, reject_target) = vm.new_capability(this)?;
    let target = vm.new_array(vec![resolve_target, reject_target]);
    let results = vm.new_array(Vec::new());
    // One more than there are, until they've all been started, so nothing finishes too soon.
    let remaining = vm.new_array(vec![Value::int(1)]);
    let outcome = (|| -> Run<()> {
        let resolver = vm.get_named(this, "resolve")?;
        if !vm.is_callable(resolver) {
            return Err(vm.type_error("Promise resolve is not a function"));
        }
        let mut place = 0usize;
        vm.iterate(source, |vm, item| {
            let state = vm.new_array(vec![
                results,
                remaining,
                target,
                Value::number(place as f64),
            ]);
            store_result(vm, results, place, Value::UNDEFINED);
            let count = vm.get_index(remaining, 0)?.as_number();
            vm.set_index(remaining, 0, Value::number(count + 1.0))?;
            place += 1;
            let (on_fulfilled, on_rejected) = match mode {
                Combinator::All => (
                    vm.new_native(text::EMPTY_NAME, 1, all_element, state, false),
                    reject_target,
                ),
                Combinator::Settled => (
                    vm.new_native(text::EMPTY_NAME, 1, settled_fulfilled, state, false),
                    vm.new_native(text::EMPTY_NAME, 1, settled_rejected, state, false),
                ),
                Combinator::Any => (
                    resolve_target,
                    vm.new_native(text::EMPTY_NAME, 1, any_rejected, state, false),
                ),
            };
            let promise = vm.call(resolver, this, &[item])?;
            let then = vm.get(promise, text::THEN)?;
            vm.call(then, promise, &[on_fulfilled, on_rejected])?;
            Ok(())
        })?;
        let count = vm.get_index(remaining, 0)?.as_number() - 1.0;
        vm.set_index(remaining, 0, Value::number(count))?;
        if count == 0.0 {
            finish_all(vm, target, results, mode)?;
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        vm.call(reject_target, Value::UNDEFINED, &[error])?;
    }
    Ok(promise_made)
}

fn all(vm: &mut Vm, args: Args) -> Run<Value> {
    combine(vm, &args, Combinator::All)
}

fn all_settled(vm: &mut Vm, args: Args) -> Run<Value> {
    combine(vm, &args, Combinator::Settled)
}

fn any(vm: &mut Vm, args: Args) -> Run<Value> {
    combine(vm, &args, Combinator::Any)
}

fn race(vm: &mut Vm, args: Args) -> Run<Value> {
    let source = vm.arg(&args, 0);
    let this = args.this;
    let (target, resolve, reject) = vm.new_capability(this)?;
    let outcome = (|| -> Run<()> {
        let resolver = vm.get_named(this, "resolve")?;
        if !vm.is_callable(resolver) {
            return Err(vm.type_error("Promise resolve is not a function"));
        }
        vm.iterate(source, |vm, item| {
            let promise = vm.call(resolver, this, &[item])?;
            let then = vm.get(promise, text::THEN)?;
            vm.call(then, promise, &[resolve, reject])?;
            Ok(())
        })
    })();
    if let Err(error) = outcome {
        vm.call(reject, Value::UNDEFINED, &[error])?;
    }
    Ok(target)
}

fn queue_microtask(vm: &mut Vm, args: Args) -> Run<Value> {
    let function = vm.arg(&args, 0);
    if !vm.is_callable(function) {
        return Err(vm.type_error("The callback must be a function"));
    }
    vm.jobs.push_back(Job::Call(function));
    Ok(Value::UNDEFINED)
}

fn add_timer(vm: &mut Vm, args: &Args, delay: f64, repeating: bool, skip: usize) -> Run<Value> {
    let callback = vm.arg(args, 0);
    if !vm.is_callable(callback) {
        return Err(vm.type_error("The callback must be a function"));
    }
    let extra = (skip..args.count)
        .map(|index| vm.arg(args, index))
        .collect();
    vm.next_timer += 1;
    let id = vm.next_timer;
    let delay = if delay.is_nan() { 0.0 } else { delay.max(0.0) };
    let object = timer_object(vm, id, skip == 1);
    vm.timers.push(Timer {
        id,
        at: Vm::now_ms() + delay,
        callback,
        args: extra,
        repeat: repeating.then_some(delay),
        delay,
        refed: true,
        object,
    });
    Ok(object)
}

fn set_timeout(vm: &mut Vm, args: Args) -> Run<Value> {
    let delay = vm.arg(&args, 1);
    let delay = vm.to_number(delay)?;
    add_timer(vm, &args, delay, false, 2)
}

fn set_interval(vm: &mut Vm, args: Args) -> Run<Value> {
    let delay = vm.arg(&args, 1);
    let delay = vm.to_number(delay)?;
    add_timer(vm, &args, delay, true, 2)
}

fn set_immediate(vm: &mut Vm, args: Args) -> Run<Value> {
    add_timer(vm, &args, 0.0, false, 1)
}

/// `clearTimeout`, `clearInterval` and `clearImmediate`, which take what made the timer gave, or its number, as `+t`
/// gives it, or that number as text, as Node does.
fn clear_timer(vm: &mut Vm, args: Args) -> Run<Value> {
    let given = vm.arg(&args, 0);
    let id = if given.is_object() {
        timer_id(vm, given)
    } else if given.is_number() || given.is_string() {
        let number = vm.to_number(given)?;
        (number.fract() == 0.0 && number >= 0.0 && number <= f64::from(u32::MAX)).then_some(number as u32)
    } else {
        None
    };
    if let Some(id) = id {
        vm.timers.retain(|timer| timer.id != id);
    }
    Ok(Value::UNDEFINED)
}

// What `setTimeout` and `setInterval` give, which Node calls a `Timeout`, and what `setImmediate` gives, an
// `Immediate`. Each says which timer it is by its number, which is what `+t` gives, so that what's written for a page,
// where timers are numbers, still works. A timer that's been unref'd still runs, but doesn't keep the program going
// by itself (`Engine::run_until_idle`).

const TIMER_KEY: &str = "%timer";
/// What a timer that's run once was, kept on its object for `refresh`: its callback, its delay, and its arguments.
const SPENT_KEY: &str = "%spentTimer";
/// Set on a timer's object while it's unref'd, so `hasRef` still says so once it's run.
const UNREF_KEY: &str = "%unrefTimer";

fn timer_object(vm: &mut Vm, id: u32, immediate: bool) -> Value {
    let proto = timer_proto(vm, immediate);
    let object = vm.new_object_with_proto(proto, Kind::Plain);
    let key = vm.private_atom(TIMER_KEY);
    vm.add_property(object.handle(), key, Value::number(f64::from(id)), HIDDEN);
    object
}

/// The prototype of each kind, made the first time a timer is.
fn timer_proto(vm: &mut Vm, immediate: bool) -> u32 {
    let made = if immediate { vm.realm.immediate_proto.0 } else { vm.realm.timeout_proto.0 };
    if made.is_object() {
        return made.handle();
    }
    let proto = vm.plain_proto();
    let name = vm.intern_str(if immediate { "Immediate" } else { "Timeout" });
    let constructor = vm.new_native(name, 0, illegal_timer_constructor, Value::UNDEFINED, true);
    vm.add_property(constructor.handle(), text::PROTOTYPE, Value::object(proto), 0);
    vm.add_property(proto, text::CONSTRUCTOR, constructor, HIDDEN);
    vm.method(proto, "hasRef", 0, timer_has_ref);
    vm.method(proto, "ref", 0, timer_ref);
    vm.method(proto, "unref", 0, timer_unref);
    if immediate {
        vm.realm.immediate_proto = crate::vm::Value0(Value::object(proto));
    } else {
        vm.method(proto, "refresh", 0, timer_refresh);
        vm.method(proto, "close", 0, timer_close);
        vm.symbol_method(proto, text::SYMBOL_TO_PRIMITIVE, "[Symbol.toPrimitive]", 0, timer_to_primitive);
        vm.realm.timeout_proto = crate::vm::Value0(Value::object(proto));
    }
    proto
}

fn illegal_timer_constructor(vm: &mut Vm, _: Args) -> Run<Value> {
    Err(vm.type_error("Illegal constructor"))
}

/// The number of the timer an object made by `setTimeout` or the like is for.
fn timer_id(vm: &mut Vm, value: Value) -> Option<u32> {
    if !value.is_object() {
        return None;
    }
    let key = vm.private_atom(TIMER_KEY);
    let found = vm.peek_property(value.handle(), key)?;
    found.is_number().then(|| found.as_number() as u32)
}

fn hidden_flag(vm: &mut Vm, object: Value, name: &str, on: bool) {
    let key = vm.private_atom(name);
    vm.define(object.handle(), key, Value::bool(on), HIDDEN);
}

fn timer_has_ref(vm: &mut Vm, args: Args) -> Run<Value> {
    let Some(id) = timer_id(vm, args.this) else {
        return Ok(Value::FALSE);
    };
    if let Some(timer) = vm.timers.iter().find(|timer| timer.id == id) {
        return Ok(Value::bool(timer.refed));
    }
    let key = vm.private_atom(UNREF_KEY);
    let unrefed = vm.peek_property(args.this.handle(), key) == Some(Value::TRUE);
    Ok(Value::bool(!unrefed))
}

fn set_refed(vm: &mut Vm, args: &Args, refed: bool) -> Run<Value> {
    if let Some(id) = timer_id(vm, args.this) {
        if let Some(timer) = vm.timers.iter_mut().find(|timer| timer.id == id) {
            timer.refed = refed;
        }
        hidden_flag(vm, args.this, UNREF_KEY, !refed);
    }
    Ok(args.this)
}

fn timer_ref(vm: &mut Vm, args: Args) -> Run<Value> {
    set_refed(vm, &args, true)
}

fn timer_unref(vm: &mut Vm, args: Args) -> Run<Value> {
    set_refed(vm, &args, false)
}

/// Starts the timer's wait again from now, and has a timer that's already run once run again, as Node does.
fn timer_refresh(vm: &mut Vm, args: Args) -> Run<Value> {
    let Some(id) = timer_id(vm, args.this) else {
        return Ok(args.this);
    };
    let now = Vm::now_ms();
    if let Some(timer) = vm.timers.iter_mut().find(|timer| timer.id == id) {
        timer.at = now + timer.delay;
        return Ok(args.this);
    }
    let key = vm.private_atom(SPENT_KEY);
    let Some(spent) = vm.peek_property(args.this.handle(), key).filter(|spent| spent.is_object()) else {
        return Ok(args.this);
    };
    let items = vm.array_items(spent);
    let (callback, delay) = (items[0], items[1].as_number());
    let unref_key = vm.private_atom(UNREF_KEY);
    let refed = vm.peek_property(args.this.handle(), unref_key) != Some(Value::TRUE);
    vm.timers.push(Timer {
        id,
        at: now + delay,
        callback,
        args: items[2..].to_vec(),
        repeat: None,
        delay,
        refed,
        object: args.this,
    });
    Ok(args.this)
}

fn timer_close(vm: &mut Vm, args: Args) -> Run<Value> {
    if let Some(id) = timer_id(vm, args.this) {
        vm.timers.retain(|timer| timer.id != id);
    }
    Ok(args.this)
}

fn timer_to_primitive(vm: &mut Vm, args: Args) -> Run<Value> {
    Ok(timer_id(vm, args.this).map_or(Value::UNDEFINED, |id| Value::number(f64::from(id))))
}

impl Vm {
    /// Whether a timer that hasn't been unref'd is waiting, which keeps the program going.
    pub(crate) fn timers_keep_alive(&self) -> bool {
        self.timers.iter().any(|timer| timer.refed)
    }

    /// Keeps what a timer that's run once was on its object, which `refresh` can run again.
    fn keep_spent_timer(&mut self, timer: &Timer) {
        if !timer.object.is_object() {
            return;
        }
        let mut items = vec![timer.callback, Value::number(timer.delay)];
        items.extend_from_slice(&timer.args);
        let spent = self.new_array(items);
        let key = self.private_atom(SPENT_KEY);
        self.define(timer.object.handle(), key, spent, HIDDEN);
    }
}
