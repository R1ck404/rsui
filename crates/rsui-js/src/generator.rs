//! Generators and async functions, which are both functions that can stop partway through and carry on later.

use crate::object::*;
use crate::text;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Suspended {
    /// It ran to its end.
    None,
    Yield,
    Await,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Resume {
    Next,
    Throw,
    Return,
}

/// Where a generator's value goes when it's first started: nowhere.
pub(crate) const NOT_STARTED: u16 = u16::MAX;

impl Vm {
    /// Carries on with a generator until it next stops, and says how it stopped.
    pub(crate) fn resume_generator(
        &mut self,
        generator: Value,
        mode: Resume,
        value: Value,
    ) -> Run<(Suspended, Value)> {
        let handle = generator.handle();
        let Kind::Generator(data) = &mut self.objects[handle as usize].kind else {
            unreachable!("only generators are resumed");
        };
        data.state = GeneratorState::Running;
        let resume_register = data.resume_register;
        if mode == Resume::Return {
            data.return_value = value;
        }
        let stop = self.frames.len();
        self.enter_generator_frame(handle)?;
        let saved_current = self.current_generator;
        self.current_generator = handle;
        self.yielded = Suspended::None;
        let result = match mode {
            Resume::Next => {
                if resume_register != NOT_STARTED {
                    self.set_register(resume_register, value);
                }
                self.run(stop)
            }
            Resume::Throw | Resume::Return => {
                let error = if mode == Resume::Throw {
                    value
                } else {
                    Value::GENERATOR_RETURN
                };
                self.throw_into_frame(error, stop)
            }
        };
        self.current_generator = saved_current;
        let how = std::mem::replace(&mut self.yielded, Suspended::None);
        let Kind::Generator(data) = &mut self.objects[handle as usize].kind else {
            unreachable!("only generators are resumed");
        };
        match result {
            Ok(value) => {
                data.state = if how == Suspended::None {
                    GeneratorState::Done
                } else {
                    GeneratorState::Suspended
                };
                Ok((how, value))
            }
            Err(error) => {
                data.state = GeneratorState::Done;
                if error == Value::GENERATOR_RETURN {
                    return Ok((Suspended::None, data.return_value));
                }
                Err(error)
            }
        }
    }

    /// The object `{ value, done }`. Its shape is always the same, so after the first it's made with it straight away.
    pub(crate) fn iter_result(&mut self, value: Value, done: bool) -> Value {
        let result = self.new_plain();
        if self.iter_result_shape != NONE {
            let made = &mut self.objects[result.index()];
            made.set_slots(&[value, Value::bool(done)]);
            made.shape = self.iter_result_shape;
            return result;
        }
        self.define(result.handle(), text::VALUE, value, PLAIN);
        self.define(result.handle(), text::DONE, Value::bool(done), PLAIN);
        self.iter_result_shape = self.objects[result.index()].shape;
        result
    }

    /// The next value of a generator that `for of` or a spread is going through, if its `next` is the built-in one:
    /// then there's no `{ value, done }` to make and read back. Nothing if it's some other iterator.
    pub(crate) fn generator_step_directly(
        &mut self,
        iterator: Value,
        next: Value,
    ) -> Option<Run<Option<Value>>> {
        if !next.is_object() || !iterator.is_object() {
            return None;
        }
        match &self.objects[next.index()].kind {
            Kind::Native(native) if native.func as usize == Self::generator_next as NativeFn as usize => {}
            _ => return None,
        }
        let Kind::Generator(data) = &self.objects[iterator.index()].kind else {
            return None;
        };
        Some(match data.state {
            GeneratorState::Running => Err(self.type_error("Generator is already running")),
            GeneratorState::Done => Ok(None),
            GeneratorState::Suspended => {
                match self.resume_generator(iterator, Resume::Next, Value::UNDEFINED) {
                    Ok((Suspended::None, _)) => Ok(None),
                    Ok((_, value)) => Ok(Some(value)),
                    Err(error) => Err(error),
                }
            }
        })
    }

    fn generator_step(&mut self, args: &Args, mode: Resume) -> Run<Value> {
        let generator = args.this;
        let value = self.arg(args, 0);
        let state = if generator.is_object() {
            match &self.objects[generator.index()].kind {
                Kind::Generator(data) => Some((data.state, data.resume_register)),
                _ => None,
            }
        } else {
            None
        };
        let Some((state, register)) = state else {
            return Err(self.type_error("next method called on incompatible receiver"));
        };
        match state {
            GeneratorState::Running => {
                return Err(self.type_error("Generator is already running"));
            }
            GeneratorState::Done => {
                return match mode {
                    Resume::Next => Ok(self.iter_result(Value::UNDEFINED, true)),
                    Resume::Return => Ok(self.iter_result(value, true)),
                    Resume::Throw => Err(value),
                };
            }
            GeneratorState::Suspended => {}
        }
        // One that hasn't started has nothing to clean up.
        if register == NOT_STARTED && mode != Resume::Next {
            if let Kind::Generator(data) = &mut self.objects[generator.index()].kind {
                data.state = GeneratorState::Done;
            }
            return match mode {
                Resume::Return => Ok(self.iter_result(value, true)),
                _ => Err(value),
            };
        }
        let (how, result) = self.resume_generator(generator, mode, value)?;
        Ok(self.iter_result(result, how == Suspended::None))
    }

    pub(crate) fn generator_next(vm: &mut Vm, args: Args) -> Run<Value> {
        vm.generator_step(&args, Resume::Next)
    }

    pub(crate) fn generator_return(vm: &mut Vm, args: Args) -> Run<Value> {
        vm.generator_step(&args, Resume::Return)
    }

    pub(crate) fn generator_throw(vm: &mut Vm, args: Args) -> Run<Value> {
        vm.generator_step(&args, Resume::Throw)
    }

    /// Runs an async function up to its first `await`, and gives what its caller gets: a promise.
    pub(crate) fn start_async(&mut self, generator: Value) -> Value {
        let promise = self.new_promise();
        if let Kind::Generator(data) = &mut self.objects[generator.index()].kind {
            data.promise = promise;
        }
        self.async_step(generator, Resume::Next, Value::UNDEFINED);
        promise
    }

    /// Carries on an async function or generator that was waiting for a promise, now that it's settled. What waits for
    /// a promise is the generator itself, rather than a function that carries it on: the promise is always one of the
    /// engine's own, so nothing else sees what it waits with, and `Vm::run_job` sends it here.
    pub(crate) fn resume_waiting(&mut self, generator: Value, rejected: bool, value: Value) {
        let mode = if rejected {
            Resume::Throw
        } else {
            Resume::Next
        };
        let is_async_function = matches!(
            &self.objects[generator.index()].kind,
            Kind::Generator(data) if !data.promise.is_undefined()
        );
        if is_async_function {
            self.async_step(generator, mode, value);
        } else {
            self.async_generator_step(generator, mode, value);
        }
    }

    fn async_step(&mut self, generator: Value, mode: Resume, value: Value) {
        let promise = match &self.objects[generator.index()].kind {
            Kind::Generator(data) => data.promise,
            _ => unreachable!("only async functions come here"),
        };
        match self.resume_generator(generator, mode, value) {
            Ok((Suspended::Await, awaited)) => {
                let waiting = match self.promise_resolve_value(awaited) {
                    Ok(waiting) => waiting,
                    Err(error) => {
                        self.async_step(generator, Resume::Throw, error);
                        return;
                    }
                };
                self.promise_then(waiting, generator, generator, Value::UNDEFINED);
            }
            Ok((_, result)) => self.resolve_promise(promise, result),
            Err(error) => self.reject_promise(promise, error),
        }
    }

    // Async generators: each call waits its turn, and the turn after it starts when it's done.

    fn async_generator_call(vm: &mut Vm, args: Args, mode: Resume) -> Run<Value> {
        let promise = vm.new_promise();
        let is_generator = args.this.is_object()
            && matches!(vm.objects[args.this.index()].kind, Kind::Generator(_));
        if !is_generator {
            let error = vm.type_error("next method called on incompatible receiver");
            vm.reject_promise(promise, error);
            return Ok(promise);
        }
        let value = vm.arg(&args, 0);
        let Kind::Generator(data) = &mut vm.objects[args.this.index()].kind else {
            unreachable!("just checked");
        };
        data.queue.push((mode, value, promise));
        let idle = data.state != GeneratorState::Running && data.queue.len() == 1;
        if idle {
            vm.async_generator_next_turn(args.this);
        }
        Ok(promise)
    }

    pub(crate) fn async_generator_next(vm: &mut Vm, args: Args) -> Run<Value> {
        Self::async_generator_call(vm, args, Resume::Next)
    }

    pub(crate) fn async_generator_return(vm: &mut Vm, args: Args) -> Run<Value> {
        Self::async_generator_call(vm, args, Resume::Return)
    }

    pub(crate) fn async_generator_throw(vm: &mut Vm, args: Args) -> Run<Value> {
        Self::async_generator_call(vm, args, Resume::Throw)
    }

    fn async_generator_next_turn(&mut self, generator: Value) {
        loop {
            let Kind::Generator(data) = &mut self.objects[generator.index()].kind else {
                return;
            };
            let Some((mode, value, promise)) = data.queue.first().copied() else {
                return;
            };
            if data.resume_register == NOT_STARTED && mode != Resume::Next {
                data.state = GeneratorState::Done;
            }
            if data.state != GeneratorState::Done {
                self.async_generator_step(generator, mode, value);
                return;
            }
            data.queue.remove(0);
            match mode {
                Resume::Next => {
                    let result = self.iter_result(Value::UNDEFINED, true);
                    self.resolve_promise(promise, result);
                }
                Resume::Return => {
                    let result = self.iter_result(value, true);
                    self.resolve_promise(promise, result);
                }
                Resume::Throw => self.reject_promise(promise, value),
            }
        }
    }

    /// Finishes the call at the front of the queue, and starts the next.
    fn async_generator_settle(&mut self, generator: Value, outcome: Result<(Value, bool), Value>) {
        let Kind::Generator(data) = &mut self.objects[generator.index()].kind else {
            return;
        };
        if data.queue.is_empty() {
            return;
        }
        let (_, _, promise) = data.queue.remove(0);
        match outcome {
            Ok((value, done)) => {
                let result = self.iter_result(value, done);
                self.resolve_promise(promise, result);
            }
            Err(error) => self.reject_promise(promise, error),
        }
        self.async_generator_next_turn(generator);
    }

    fn async_generator_step(&mut self, generator: Value, mode: Resume, value: Value) {
        match self.resume_generator(generator, mode, value) {
            Ok((Suspended::Await, awaited)) => {
                // It's not to be started again until what it's waiting for comes.
                if let Kind::Generator(data) = &mut self.objects[generator.index()].kind {
                    data.state = GeneratorState::Running;
                }
                let waiting = match self.promise_resolve_value(awaited) {
                    Ok(waiting) => waiting,
                    Err(error) => {
                        self.async_generator_step(generator, Resume::Throw, error);
                        return;
                    }
                };
                self.promise_then(waiting, generator, generator, Value::UNDEFINED);
            }
            Ok((Suspended::Yield, yielded)) => {
                self.async_generator_settle(generator, Ok((yielded, false)));
            }
            Ok((Suspended::None, returned)) => {
                self.async_generator_settle(generator, Ok((returned, true)));
            }
            Err(error) => self.async_generator_settle(generator, Err(error)),
        }
    }

    /// What `for await` goes through. Anything iterable will do, if it's not an async one.
    pub(crate) fn get_async_iterator(&mut self, source: Value) -> Run<(Value, Value)> {
        let method = if source.is_nullish() {
            Value::UNDEFINED
        } else {
            self.get(source, text::SYMBOL_ASYNC_ITERATOR)?
        };
        if method.is_nullish() {
            let method = if source.is_nullish() {
                Value::UNDEFINED
            } else {
                self.get(source, text::SYMBOL_ITERATOR)?
            };
            if !self.is_callable(method) {
                let shown = self.brief(source);
                return Err(self.type_error(&format!("{shown} is not async iterable")));
            }
            let iterator = self.call(method, source, &[])?;
            let next = self.get(iterator, text::NEXT)?;
            return Ok((iterator, next));
        }
        let iterator = self.call(method, source, &[])?;
        let next = self.get(iterator, text::NEXT)?;
        Ok((iterator, next))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    fn run(source: &str) -> String {
        let output = Rc::new(RefCell::new(String::new()));
        let mut engine = crate::Engine::new();
        let sink = output.clone();
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        if let Err(error) = engine
            .eval(source, "test.js")
            .and_then(|_| engine.run_until_idle())
        {
            panic!("{source}\nthrew: {error}");
        }
        output.borrow().trim_end().to_owned()
    }

    #[test]
    fn going_through_a_generator_skips_its_result_objects_but_not_what_they_mean() {
        assert_eq!(
            run("function* g() { yield 1; yield 2; return 3 } const out = []; for (const v of g()) out.push(v); console.log(out.join(), [...g()].join(), Array.from(g()).join())"),
            "1,2 1,2 1,2"
        );
        // A `next` of its own is still what's called.
        assert_eq!(
            run("function* g() { yield 1 } const it = g(); it.next = () => ({ value: 7, done: it.n++ > 0 }); it.n = 0; console.log([...it].join())"),
            "7"
        );
        assert_eq!(
            run("function* g() { yield 1 } const proto = Object.getPrototypeOf(g.prototype); const next = proto.next; proto.next = function () { const r = next.call(this); if (!r.done) r.value *= 10; return r }; console.log([...g()].join()); proto.next = next"),
            "10"
        );
        assert_eq!(
            run("let it; function* g() { yield 1; try { for (const v of it) {} } catch (e) { yield e.constructor.name } } it = g(); console.log([...it].join())"),
            "1,TypeError"
        );
        assert_eq!(
            run("function* g() { try { yield 1; yield 2 } finally { console.log('closed') } } const [a] = g(); console.log(a); const it = g(); for (const v of it) break; console.log(it.next().done)"),
            "closed\n1\nclosed\ntrue"
        );
        assert_eq!(
            run("function* g() { yield 1; throw new Error('no') } try { for (const v of g()) console.log(v) } catch (e) { console.log(e.message) }"),
            "1\nno"
        );
    }

    #[test]
    fn bitwise_operators_wrap_doubles_as_they_should() {
        assert_eq!(
            run("console.log((2**32 + 5) | 0, 2**31 | 0, (-(2**31) - 1) | 0, 1e20 | 0, -1e20 | 0, NaN | 0, Infinity | 0, -Infinity >> 1)"),
            "5 -2147483648 2147483647 1661992960 -1661992960 0 0 0"
        );
        assert_eq!(
            run("console.log(4294967295.7 >>> 0, ~(2**32 + 1), -1.5 | 0, 2**53 ^ 1, (2**63) | 0, (2**62 + 2**31) & -1, 1.9 << 1, ~1.5, ~(2**40))"),
            "4294967295 -2 -1 1 0 -2147483648 2 -2 -1"
        );
    }
}
