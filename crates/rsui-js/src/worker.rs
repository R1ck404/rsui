//! Code on another thread, with an engine of its own. Nothing's shared between engines, so what goes from one to the other is written
//! out as [`Copied`] on one side, and made into values on the other, as `structuredClone` would.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use crate::background::{Later, Native, Outcome};
use crate::builtins::host::{bytes_of, new_bytes};
use crate::object::{Kind, PLAIN};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

/// How much of its stack a worker's engine uses, and how much it has. It only takes memory as it's used.
const STACK: usize = 64 * 1024 * 1024;

/// A value, away from any engine.
#[derive(Debug, Clone)]
pub(crate) enum Copied {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Copied>),
    Object(Vec<(String, Copied)>),
    Map(Vec<(Copied, Copied)>),
    Set(Vec<Copied>),
    Date(f64),
    Buffer(Vec<u8>),
    /// Bytes as one of the kinds of view of them, with its name.
    View(String, Vec<u8>),
    /// Something that's been seen already, as the nth thing that was. That's how cycles stay cycles.
    Again(usize),
}

fn to_copied(vm: &mut Vm, value: Value, seen: &mut Vec<Value>) -> Run<Copied> {
    if value.is_undefined() {
        return Ok(Copied::Undefined);
    }
    if value.is_nullish() {
        return Ok(Copied::Null);
    }
    if value.is_bool() {
        return Ok(Copied::Bool(value == Value::TRUE));
    }
    if value.is_number() {
        return Ok(Copied::Number(value.as_number()));
    }
    if value.is_string() {
        return Ok(Copied::Text(vm.to_rust_string(value)?));
    }
    if !value.is_object() {
        return Err(vm.make_error(vm.realm.error_proto, "That could not be cloned"));
    }
    if let Some(at) = seen.iter().position(|one| *one == value) {
        return Ok(Copied::Again(at));
    }
    if vm.is_callable(value) {
        return Err(vm.make_error(vm.realm.error_proto, "A function could not be cloned"));
    }
    match &vm.objects[value.index()].kind {
        Kind::Array(_) => {
            seen.push(value);
            let items = vm.array_items(value);
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(to_copied(vm, item, seen)?);
            }
            Ok(Copied::Array(out))
        }
        Kind::Date(time) => {
            let time = *time;
            seen.push(value);
            Ok(Copied::Date(time))
        }
        Kind::Map(data) => {
            let entries = data.entries.clone();
            seen.push(value);
            let mut out = Vec::with_capacity(entries.len());
            for (key, item) in entries {
                if !key.is_empty() {
                    out.push((to_copied(vm, key, seen)?, to_copied(vm, item, seen)?));
                }
            }
            Ok(Copied::Map(out))
        }
        Kind::Set(data) => {
            let entries = data.entries.clone();
            seen.push(value);
            let mut out = Vec::with_capacity(entries.len());
            for (key, _) in entries {
                if !key.is_empty() {
                    out.push(to_copied(vm, key, seen)?);
                }
            }
            Ok(Copied::Set(out))
        }
        Kind::ArrayBuffer(data) => {
            let bytes = data.bytes.clone();
            seen.push(value);
            Ok(Copied::Buffer(bytes))
        }
        Kind::TypedArray(_) | Kind::DataView(_) => {
            let bytes = bytes_of(vm, value)?;
            let constructor = vm.get_named(value, "constructor")?;
            let name = vm.get_named(constructor, "name")?;
            let name = vm.to_rust_string(name)?;
            seen.push(value);
            Ok(Copied::View(name, bytes))
        }
        _ => {
            seen.push(value);
            let names = vm.own_enumerable_names(value.handle());
            let mut out = Vec::with_capacity(names.len());
            for name in names {
                let item = vm.get(value, name)?;
                let key = vm.atom_text(name);
                out.push((key, to_copied(vm, item, seen)?));
            }
            Ok(Copied::Object(out))
        }
    }
}

fn from_copied(vm: &mut Vm, copied: &Copied, made: &mut Vec<Value>) -> Run<Value> {
    Ok(match copied {
        Copied::Undefined => Value::UNDEFINED,
        Copied::Null => Value::NULL,
        Copied::Bool(value) => Value::bool(*value),
        Copied::Number(value) => Value::number(*value),
        Copied::Text(text) => vm.new_str(text),
        Copied::Again(at) => made.get(*at).copied().unwrap_or(Value::UNDEFINED),
        Copied::Array(items) => {
            let out = vm.new_array(vec![Value::UNDEFINED; items.len()]);
            made.push(out);
            for (index, item) in items.iter().enumerate() {
                let inner = from_copied(vm, item, made)?;
                vm.set_index(out, index, inner)?;
            }
            out
        }
        Copied::Object(fields) => {
            let out = vm.new_plain();
            made.push(out);
            for (name, item) in fields {
                let inner = from_copied(vm, item, made)?;
                let name = vm.intern_key_str(name);
                vm.define(out.handle(), name, inner, PLAIN);
            }
            out
        }
        Copied::Date(time) => {
            let proto = vm.realm.date_proto;
            let out = vm.new_object_with_proto(proto, Kind::Date(*time));
            made.push(out);
            out
        }
        Copied::Map(entries) => {
            let constructor = vm.get_named(Value::object(vm.global), "Map")?;
            let out = vm.construct(constructor, &[])?;
            made.push(out);
            let set = vm.get_named(out, "set")?;
            for (key, item) in entries {
                let key = from_copied(vm, key, made)?;
                let item = from_copied(vm, item, made)?;
                vm.call(set, out, &[key, item])?;
            }
            out
        }
        Copied::Set(items) => {
            let constructor = vm.get_named(Value::object(vm.global), "Set")?;
            let out = vm.construct(constructor, &[])?;
            made.push(out);
            let add = vm.get_named(out, "add")?;
            for item in items {
                let item = from_copied(vm, item, made)?;
                vm.call(add, out, &[item])?;
            }
            out
        }
        Copied::Buffer(bytes) => {
            let out = vm.new_array_buffer(bytes.clone());
            made.push(out);
            out
        }
        Copied::View(name, bytes) => {
            let plain = new_bytes(vm, bytes.clone())?;
            let buffer = vm.get_named(plain, "buffer")?;
            let constructor = vm.get_named(Value::object(vm.global), name)?;
            // Something that's made a kind of its own has lost that by here.
            let out = if constructor.is_undefined() {
                plain
            } else {
                vm.construct(constructor, &[buffer])?
            };
            made.push(out);
            out
        }
    })
}

/// What goes from one side to the other.
#[derive(Debug, Clone)]
pub(crate) enum Letter {
    /// It's started.
    Online,
    Message(Copied),
    /// It went wrong, and this is what to say.
    Failed(String),
    /// It's finished, with this code.
    Exited(i32),
}

/// Letters on their way to one side, or what's waiting for them there. Posting one to something that's waiting keeps its
/// promise there and then, so nothing needs a thread to wait.
#[derive(Default)]
struct Mailbox {
    inside: Mutex<Inside>,
}

#[derive(Default)]
struct Inside {
    letters: VecDeque<Letter>,
    waiting: VecDeque<Later>,
    closed: bool,
}

impl Mailbox {
    fn post(&self, letter: Letter) {
        let later = {
            let mut inside = crate::lock(&self.inside);
            match inside.waiting.pop_front() {
                Some(later) => later,
                None => {
                    inside.letters.push_back(letter);
                    return;
                }
            }
        };
        later.settle(Outcome::Letter(letter));
    }

    fn close(&self) {
        let waiting = {
            let mut inside = crate::lock(&self.inside);
            inside.closed = true;
            std::mem::take(&mut inside.waiting)
        };
        for later in waiting {
            later.settle(Outcome::Null);
        }
    }

    /// What's next, if it's here, or `null` if nothing more can come. Otherwise `later` is kept until then.
    fn next(&self, later: Later) {
        let mut inside = crate::lock(&self.inside);
        let outcome = match inside.letters.pop_front() {
            Some(letter) => Outcome::Letter(letter),
            None if inside.closed => Outcome::Null,
            None => {
                inside.waiting.push_back(later);
                return;
            }
        };
        drop(inside);
        later.settle(outcome);
    }
}

impl Drop for Mailbox {
    /// Nothing more can come to what's still waiting.
    fn drop(&mut self) {
        self.close();
    }
}

/// What's between a program and a worker.
#[derive(Default)]
struct Link {
    id: u32,
    to_worker: Mailbox,
    to_parent: Mailbox,
    stop: AtomicBool,
    /// Which of the worker's engine's waits to wake up, when it's asleep.
    awake: (Mutex<bool>, Condvar),
    data: Option<Copied>,
}

impl Link {
    fn wake(&self) {
        if let Ok(mut awake) = self.awake.0.lock() {
            *awake = true;
        }
        self.awake.1.notify_all();
    }
}

fn workers() -> &'static Mutex<HashMap<u32, Arc<Link>>> {
    static ALL: OnceLock<Mutex<HashMap<u32, Arc<Link>>>> = OnceLock::new();
    ALL.get_or_init(Mutex::default)
}

thread_local! {
    /// What this thread's engine is a worker for, if it's one.
    static THIS: std::cell::RefCell<Option<Arc<Link>>> = const { std::cell::RefCell::new(None) };
}

fn this_link() -> Option<Arc<Link>> {
    THIS.with(|this| this.borrow().clone())
}

fn link(number: u32) -> Option<Arc<Link>> {
    workers().lock().ok()?.get(&number).cloned()
}

/// The files a worker started with `file` reads as it starts, which a packed program has to have in it as well. It's
/// started on a thread of its own, as it would be, and let go of once it's waiting for its first message.
pub fn files_a_worker_loads(file: &Path) -> Vec<PathBuf> {
    let file = file.to_owned();
    let started = std::thread::Builder::new()
        .name("rsui worker, for packing".into())
        .stack_size(STACK)
        .spawn(move || {
            let mut engine = crate::Engine::new();
            engine.set_stack_limit(STACK / 8 * 7);
            if engine.load_worker_globals().is_err() {
                return Vec::new();
            }
            let _ = engine.run_file(&file);
            engine.loaded_files()
        });
    started.ok().and_then(|thread| thread.join().ok()).unwrap_or_default()
}

fn run_worker(link: &Arc<Link>, file: &Path) -> Result<(), String> {
    let mut engine = crate::Engine::new();
    engine.set_stack_limit(STACK / 8 * 7);
    for (name, value) in std::env::vars() {
        engine.set_env(&name, &value);
    }
    let waking = link.clone();
    engine.set_waker(move || waking.wake());
    let failed = |error: crate::JsError| error.message;
    // That's what `self`, `postMessage` and the rest are for.
    engine.load_worker_globals().map_err(failed)?;
    engine.run_file(file).map_err(failed)?;
    loop {
        engine.run_jobs().map_err(failed)?;
        let wait = engine.run_timers().map_err(failed)?;
        engine.run_jobs().map_err(failed)?;
        if link.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if engine.has_ready_work() {
            continue;
        }
        if (wait.is_none() || !engine.timers_keep_alive()) && !engine.is_working() {
            return Ok(());
        }
        let longest = wait.map_or(Duration::from_secs(3600), |wait| {
            Duration::from_secs_f64(wait.max(0.0) / 1000.0)
        });
        let Ok(mut awake) = link.awake.0.lock() else {
            return Ok(());
        };
        if !*awake && !longest.is_zero() {
            if let Ok((woken, _)) = link.awake.1.wait_timeout(awake, longest) {
                awake = woken;
            } else {
                return Ok(());
            }
        }
        *awake = false;
    }
}

/// `workerStart(file, whether there's data, data)`: its number.
fn worker_start(vm: &mut Vm, args: Args) -> Run<Value> {
    let file = vm.arg(&args, 0);
    let file = PathBuf::from(vm.to_rust_string(file)?);
    let data = if vm.arg(&args, 1) == Value::TRUE {
        let given = vm.arg(&args, 2);
        Some(to_copied(vm, given, &mut Vec::new())?)
    } else {
        None
    };
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let made = Arc::new(Link {
        id: number,
        data,
        ..Link::default()
    });
    if let Ok(mut all) = workers().lock() {
        all.insert(number, made.clone());
    }
    let started = std::thread::Builder::new()
        .name("rsui worker".into())
        .stack_size(STACK)
        .spawn(move || {
            THIS.with(|this| *this.borrow_mut() = Some(made.clone()));
            made.to_parent.post(Letter::Online);
            let code = match run_worker(&made, &file) {
                Ok(()) => 0,
                Err(message) => {
                    made.to_parent.post(Letter::Failed(message));
                    1
                }
            };
            made.to_worker.close();
            made.to_parent.post(Letter::Exited(code));
            made.to_parent.close();
        });
    if let Err(error) = started {
        return Err(vm.make_error(
            vm.realm.error_proto,
            &format!("The worker couldn't start: {error}"),
        ));
    }
    Ok(Value::number(f64::from(number)))
}

fn number_arg(vm: &mut Vm, args: &Args, at: usize) -> Run<u32> {
    let value = vm.arg(args, at);
    Ok(vm.to_number(value)? as u32)
}

/// `workerPost(number, message)`.
fn worker_post(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let given = vm.arg(&args, 1);
    let copied = to_copied(vm, given, &mut Vec::new())?;
    if let Some(found) = link(number) {
        found.to_worker.post(Letter::Message(copied));
        found.wake();
    }
    Ok(Value::UNDEFINED)
}

/// `parentPost(message)`, inside a worker.
fn parent_post(vm: &mut Vm, args: Args) -> Run<Value> {
    let given = vm.arg(&args, 0);
    let copied = to_copied(vm, given, &mut Vec::new())?;
    if let Some(this) = this_link() {
        this.to_parent.post(Letter::Message(copied));
    }
    Ok(Value::UNDEFINED)
}

/// `workerReceive(number)`: a promise for what's next from there, as `[kind, value]`, or `null` once there can't be any more. Inside a
/// worker, any number means what's coming from the program.
fn worker_receive(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let (found, from_parent) = match this_link() {
        Some(this) if number == 0 => (Some(this), true),
        _ => (link(number), false),
    };
    let Some(found) = found else {
        return Ok(vm.settled_promise(Outcome::Null));
    };
    let (promise, later) = vm.promise_for_later(true);
    if from_parent {
        found.to_worker.next(later);
    } else {
        found.to_parent.next(later);
    }
    Ok(promise)
}

/// `workerStop(number)`.
fn worker_stop(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    if let Some(found) = link(number) {
        found.stop.store(true, Ordering::Release);
        found.wake();
    }
    Ok(Value::UNDEFINED)
}

/// `workerForget(number)`, for one that's over.
fn worker_forget(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    if let Ok(mut all) = workers().lock() {
        all.remove(&number);
    }
    Ok(Value::UNDEFINED)
}

/// `workerClose()`, from inside a worker that's finished.
fn worker_close(_: &mut Vm, _: Args) -> Run<Value> {
    if let Some(this) = this_link() {
        this.stop.store(true, Ordering::Release);
        this.to_worker.close();
        this.wake();
    }
    Ok(Value::UNDEFINED)
}

/// `workerInfo()`: `null` on the program's own thread, and otherwise which worker this is, and what it was given.
fn worker_info(vm: &mut Vm, _: Args) -> Run<Value> {
    let Some(this) = this_link() else {
        return Ok(Value::NULL);
    };
    let out = vm.new_plain();
    let atom = vm.intern_str("id");
    vm.define(out.handle(), atom, Value::number(f64::from(this.id)), PLAIN);
    let data = match &this.data {
        Some(data) => from_copied(vm, data, &mut Vec::new())?,
        None => Value::UNDEFINED,
    };
    let atom = vm.intern_str("data");
    vm.define(out.handle(), atom, data, PLAIN);
    Ok(out)
}

/// `mainFile()`: the file that was run first, or `null` if it wasn't a file.
fn main_file(vm: &mut Vm, _: Args) -> Run<Value> {
    Ok(match vm.modules.main.clone() {
        Some(file) => vm.new_str(&file.to_string_lossy()),
        None => Value::NULL,
    })
}

pub(crate) const NATIVES: [Native; 9] = [
    ("workerStart", 3, worker_start),
    ("workerPost", 2, worker_post),
    ("parentPost", 1, parent_post),
    ("workerReceive", 1, worker_receive),
    ("workerStop", 1, worker_stop),
    ("workerForget", 1, worker_forget),
    ("workerClose", 0, worker_close),
    ("workerInfo", 0, worker_info),
    ("mainFile", 0, main_file),
];

impl Vm {
    /// What a letter is in JavaScript: `[kind, value]`.
    pub(crate) fn letter_value(&mut self, letter: Letter) -> Run<Value> {
        let (kind, value) = match letter {
            Letter::Online => ("online", Value::UNDEFINED),
            Letter::Message(copied) => ("message", from_copied(self, &copied, &mut Vec::new())?),
            Letter::Failed(message) => ("error", self.new_str(&message)),
            Letter::Exited(code) => ("exit", Value::number(f64::from(code))),
        };
        let kind = self.new_str(kind);
        Ok(self.new_array(vec![kind, value]))
    }
}
