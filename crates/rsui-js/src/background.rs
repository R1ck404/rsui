//! Work that happens on another thread while JavaScript gets on with things, and comes back as a promise being kept:
//! files and network requests. There are a few threads at most, which go away when there's nothing for them to do, so
//! a program that doesn't use this has nothing running.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::embed::Root;
use crate::object::PLAIN;
use crate::value::Value;
use crate::vm::{Run, Vm};

const MOST_WORKERS: usize = 4;
const WORKER_STACK: usize = 512 * 1024;

/// What's being waited for has this to say for itself. Nothing in here belongs to JavaScript, as it comes from another
/// thread.
// Responses only come from Windows' requests until other systems have theirs.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum Outcome {
    Nothing,
    Null,
    /// Something went wrong, and this is what to say.
    Failed(String),
    Bool(bool),
    Number(f64),
    Text(String),
    Bytes(Vec<u8>),
    Names(Vec<String>),
    Stat {
        file: bool,
        folder: bool,
        size: f64,
        modified: f64,
    },
    Response(Box<Response>),
    /// Something to do with a file went wrong. These become errors like the ones Node gives.
    FileFailed {
        code: &'static str,
        what: &'static str,
        doing: &'static str,
        path: String,
    },
    /// A request never got an answer.
    RequestFailed(String),
    /// Something to do with a socket went wrong, with the word Node has for it.
    NetFailed {
        code: &'static str,
        message: String,
    },
    /// What came to a UDP socket.
    Datagram {
        bytes: Vec<u8>,
        address: String,
        port: u16,
    },
    /// Something from a worker, or the program it belongs to.
    Letter(crate::worker::Letter),
}

pub(crate) struct Response {
    pub status: u32,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    /// Where it ended up, after any redirects.
    pub url: String,
    pub body: Vec<u8>,
}

/// A function that a built-in module is given, with its name and how many arguments it takes.
pub(crate) type Native = (
    &'static str,
    u8,
    fn(&mut crate::vm::Vm, crate::vm::Args) -> crate::vm::Run<Value>,
);

type Work = Box<dyn FnOnce() + Send>;
type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Work>,
    workers: usize,
}

struct Shared {
    queue: Mutex<Queue>,
    /// How many have finished and are waiting to be picked up.
    ready: AtomicUsize,
    waker: Mutex<Option<Waker>>,
    /// Set when something's finished, for what's waiting on this thread to hear it.
    finished_one: Mutex<bool>,
    finishing: Condvar,
}

pub(crate) struct Background {
    shared: Arc<Shared>,
    finished: Sender<(u32, Outcome)>,
    arrived: Receiver<(u32, Outcome)>,
    waiting: HashMap<u32, Root>,
    /// How many of those are only waiting to hear something, such as a socket that could be quiet for good.
    quiet: std::collections::HashSet<u32>,
    /// How many of the quiet ones don't keep the program going either, like a watcher that's been `unref()`'d.
    unheld: std::collections::HashSet<u32>,
    next: u32,
}

impl Background {
    pub(crate) fn new() -> Self {
        let (finished, arrived) = channel();
        Self {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue::default()),
                ready: AtomicUsize::new(0),
                waker: Mutex::new(None),
                finished_one: Mutex::new(false),
                finishing: Condvar::new(),
            }),
            finished,
            arrived,
            waiting: HashMap::new(),
            quiet: std::collections::HashSet::new(),
            unheld: std::collections::HashSet::new(),
            next: 0,
        }
    }

    /// Whether something's finished and has yet to be picked up.
    pub(crate) fn any_ready(&self) -> bool {
        self.shared.ready.load(Ordering::Acquire) > 0
    }

    /// Whether anything's going on that keeps the program going.
    pub(crate) fn any_waiting(&self) -> bool {
        self.waiting.len() > self.unheld.len()
    }

    /// Whether anything's going on that's going to finish by itself, rather than only waiting to hear something.
    pub(crate) fn any_busy(&self) -> bool {
        self.waiting.len() > self.quiet.len()
    }

    /// Waits until something's finished, or for as long as `longest` says. It uses nothing while it waits.
    pub(crate) fn wait_for_something(&self, longest: Option<Duration>) {
        let until = longest.map(|longest| Instant::now() + longest);
        let Ok(mut finished_one) = self.shared.finished_one.lock() else {
            return;
        };
        while !*finished_one && !self.any_ready() {
            let waited = match until {
                None => self.shared.finishing.wait(finished_one).ok(),
                Some(until) => {
                    let left = until.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    self.shared
                        .finishing
                        .wait_timeout(finished_one, left)
                        .ok()
                        .map(|(woken, _)| woken)
                }
            };
            let Some(woken) = waited else {
                return;
            };
            finished_one = woken;
        }
        *finished_one = false;
    }

    /// Says whether a quiet promise from [`Vm::promise_for_later`] keeps the program going, by its [`Later::id`].
    pub(crate) fn hold(&mut self, id: u32, held: bool) {
        if !self.quiet.contains(&id) {
            return;
        }
        if held {
            self.unheld.remove(&id);
        } else {
            self.unheld.insert(id);
        }
    }

    /// Asks to be called when something's finished. `waker` runs on another thread.
    pub(crate) fn set_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.shared.waker.lock() {
            *slot = Some(Arc::new(waker));
        }
    }
}

/// What keeps a promise from [`Vm::promise_for_later`], once, from any thread. One that's dropped without being used
/// rejects its promise, so nothing waits for good on what's gone, such as a thread that stopped partway.
pub(crate) struct Later {
    id: u32,
    shared: Arc<Shared>,
    finished: Option<Sender<(u32, Outcome)>>,
}

impl Later {
    /// Keeps the promise with this, and wakes the engine's thread to pass it on. It says whether there was still an
    /// engine to tell.
    pub(crate) fn settle(self, outcome: Outcome) {
        self.settle_heard(outcome);
    }

    /// The same, saying whether the engine is still there to hear it.
    pub(crate) fn settle_heard(mut self, outcome: Outcome) -> bool {
        self.send(outcome)
    }

    /// What names its promise to [`Background::hold`].
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    fn send(&mut self, outcome: Outcome) -> bool {
        let Some(finished) = self.finished.take() else {
            return false;
        };
        self.shared.ready.fetch_add(1, Ordering::AcqRel);
        if finished.send((self.id, outcome)).is_err() {
            // The engine's gone, so there's nobody to tell.
            self.shared.ready.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        if let Ok(mut finished_one) = self.shared.finished_one.lock() {
            *finished_one = true;
        }
        self.shared.finishing.notify_one();
        let waker = self.shared.waker.lock().ok().and_then(|slot| slot.clone());
        if let Some(waker) = waker {
            waker();
        }
        true
    }
}

impl Drop for Later {
    fn drop(&mut self) {
        self.send(Outcome::Failed(
            "what this was waiting for stopped before it finished".to_owned(),
        ));
    }
}

fn work(shared: &Shared) {
    loop {
        let job = {
            let Ok(mut queue) = shared.queue.lock() else {
                return;
            };
            let job = queue.jobs.pop_front();
            if job.is_none() {
                queue.workers -= 1;
            }
            job
        };
        let Some(job) = job else {
            return;
        };
        job();
    }
}

impl Vm {
    /// Starts something on another thread, and gives a promise for how it turns out.
    pub(crate) fn in_background(
        &mut self,
        job: impl FnOnce() -> Outcome + Send + 'static,
    ) -> Value {
        let (promise, later) = self.promise_for_later(false);
        self.in_pool(move || later.settle(job()));
        promise
    }

    /// Runs something on one of the few threads there are for files, with no promise of its own. It's for what has to
    /// wait a moment before it can go on to what keeps a promise, such as looking up a name before connecting to it.
    /// If it can't be started, it's dropped, and so is any [`Later`] it holds, which says so.
    pub(crate) fn in_pool(&mut self, job: impl FnOnce() + Send + 'static) {
        let shared = &self.background.shared;
        let Ok(mut queue) = shared.queue.lock() else {
            return;
        };
        queue.jobs.push_back(Box::new(job));
        if queue.workers < MOST_WORKERS {
            queue.workers += 1;
            let shared = shared.clone();
            let started = std::thread::Builder::new()
                .name("rsui background".into())
                .stack_size(WORKER_STACK)
                .spawn(move || work(&shared));
            if started.is_err() {
                queue.workers -= 1;
            }
        }
    }

    /// A promise that's already kept, or broken, by what's known right now.
    pub(crate) fn settled_promise(&mut self, outcome: Outcome) -> Value {
        let promise = self.new_promise();
        match self.outcome_value(outcome) {
            Ok(value) => self.resolve_promise(promise, value),
            Err(error) => self.reject_promise(promise, error),
        }
        promise
    }

    /// The same, for what can take as long as it likes, like waiting for another program. It has a thread of its own,
    /// rather than holding up one of the few there are for files.
    pub(crate) fn in_own_thread(
        &mut self,
        job: impl FnOnce() -> Outcome + Send + 'static,
    ) -> Value {
        let (promise, later) = self.promise_for_later(false);
        let _ = std::thread::Builder::new()
            .name("rsui waiting".into())
            .stack_size(WORKER_STACK)
            .spawn(move || later.settle(job()));
        promise
    }

    /// The same, for only waiting to hear something, which could never happen. What's checking that an app has settled doesn't wait for it.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn in_own_thread_quietly(
        &mut self,
        job: impl FnOnce() -> Outcome + Send + 'static,
    ) -> Value {
        let (promise, later) = self.promise_for_later(true);
        let _ = std::thread::Builder::new()
            .name("rsui waiting".into())
            .stack_size(WORKER_STACK)
            .spawn(move || later.settle(job()));
        promise
    }

    /// A promise that's kept from wherever its [`Later`] goes, on any thread, without a thread of its own. This is for
    /// what one thread looks after for many promises, such as a stream's next part or a socket's next read: it holds on
    /// to each `Later` and settles it when that's known. `quiet` is as for [`in_own_thread_quietly`](Self::in_own_thread_quietly).
    pub(crate) fn promise_for_later(&mut self, quiet: bool) -> (Value, Later) {
        let promise = self.new_promise();
        let held = self.root(promise);
        let background = &mut self.background;
        let id = background.next;
        background.next = background.next.wrapping_add(1);
        background.waiting.insert(id, held);
        if quiet {
            background.quiet.insert(id);
        }
        let later = Later {
            id,
            shared: background.shared.clone(),
            finished: Some(background.finished.clone()),
        };
        (promise, later)
    }

    /// Keeps the promises for whatever's finished.
    pub(crate) fn collect_background(&mut self) {
        while let Ok((id, outcome)) = self.background.arrived.try_recv() {
            self.background.shared.ready.fetch_sub(1, Ordering::AcqRel);
            self.background.quiet.remove(&id);
            self.background.unheld.remove(&id);
            let Some(held) = self.background.waiting.remove(&id) else {
                continue;
            };
            let promise = held.get();
            match self.outcome_value(outcome) {
                Ok(value) => self.resolve_promise(promise, value),
                Err(error) => self.reject_promise(promise, error),
            }
        }
    }

    /// What an outcome is in JavaScript: a value if it went well, and an error to throw if it didn't.
    pub(crate) fn outcome_value(&mut self, outcome: Outcome) -> Run<Value> {
        match outcome {
            Outcome::Nothing => Ok(Value::UNDEFINED),
            Outcome::Null => Ok(Value::NULL),
            Outcome::Failed(reason) => Err(self.make_error(self.realm.error_proto, &reason)),
            Outcome::Bool(value) => Ok(Value::bool(value)),
            Outcome::Number(value) => Ok(Value::number(value)),
            Outcome::Text(text) => Ok(self.new_str(&text)),
            Outcome::Bytes(bytes) => crate::builtins::host::new_bytes(self, bytes),
            Outcome::Names(names) => {
                let values = names.iter().map(|name| self.new_str(name)).collect();
                Ok(self.new_array(values))
            }
            Outcome::Stat {
                file,
                folder,
                size,
                modified,
            } => {
                let out = self.new_plain();
                for (name, value) in [
                    ("file", Value::bool(file)),
                    ("folder", Value::bool(folder)),
                    ("size", Value::number(size)),
                    ("mtimeMs", Value::number(modified)),
                ] {
                    let atom = self.intern_str(name);
                    self.define(out.handle(), atom, value, PLAIN);
                }
                Ok(out)
            }
            Outcome::Response(response) => {
                let out = self.new_plain();
                let status = Value::number(f64::from(response.status));
                let status_text = self.new_str(&response.status_text);
                let url = self.new_str(&response.url);
                let body = crate::builtins::host::new_bytes(self, response.body)?;
                let pairs: Vec<Value> = response
                    .headers
                    .iter()
                    .map(|(name, value)| {
                        let name = self.new_str(name);
                        let value = self.new_str(value);
                        self.new_array(vec![name, value])
                    })
                    .collect();
                let headers = self.new_array(pairs);
                for (name, value) in [
                    ("status", status),
                    ("statusText", status_text),
                    ("body", body),
                    ("headers", headers),
                    ("url", url),
                ] {
                    let atom = self.intern_str(name);
                    self.define(out.handle(), atom, value, PLAIN);
                }
                Ok(out)
            }
            Outcome::FileFailed {
                code,
                what,
                doing,
                path,
            } => {
                let message = format!("{code}: {what}, {doing} '{path}'");
                let made = self.make_error(self.realm.error_proto, &message);
                for (name, text) in [("code", code), ("syscall", doing), ("path", &path)] {
                    let atom = self.intern_str(name);
                    let text = self.new_str(text);
                    self.define(made.handle(), atom, text, PLAIN);
                }
                Err(made)
            }
            Outcome::NetFailed { code, message } => {
                let made = self.make_error(self.realm.error_proto, &message);
                let atom = self.intern_str("code");
                let code = self.new_str(code);
                self.define(made.handle(), atom, code, PLAIN);
                Err(made)
            }
            Outcome::Datagram {
                bytes,
                address,
                port,
            } => {
                let out = self.new_plain();
                let bytes = crate::builtins::host::new_bytes(self, bytes)?;
                let address = self.new_str(&address);
                for (name, value) in [
                    ("bytes", bytes),
                    ("address", address),
                    ("port", Value::number(f64::from(port))),
                ] {
                    let atom = self.intern_str(name);
                    self.define(out.handle(), atom, value, PLAIN);
                }
                Ok(out)
            }
            Outcome::Letter(letter) => self.letter_value(letter),
            Outcome::RequestFailed(reason) => {
                let message = format!("fetch failed: {reason}");
                Err(self.make_error(self.realm.type_error_proto, &message))
            }
        }
    }

    /// Does what an outcome says, for what was done right away.
    pub(crate) fn settle_now(&mut self, outcome: Outcome) -> Run<Value> {
        self.outcome_value(outcome)
    }
}
