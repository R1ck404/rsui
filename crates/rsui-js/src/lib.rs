//! rsui's own JavaScript engine. See `DESIGN.md`.

#![deny(unsafe_code)]
// Turning one JavaScript value into another can run a program's code, so it needs the whole engine, whatever it's called.
#![allow(clippy::wrong_self_convention)]

mod ahead;
mod arena;
mod ast;
mod background;
mod builtins;
mod census;
mod code_bytes;
mod codecache;
mod compiled;
mod compiler;
mod convert;
#[cfg(feature = "counts")]
pub mod counts;
mod embed;
mod gc;
mod generator;
#[cfg(windows)]
mod http;
mod inspect;
mod iocp;
#[cfg(feature = "jit")]
mod jit;
#[cfg(feature = "jsprof")]
mod jsprof;
mod lexer;
mod modules;
mod net;
mod object;
mod op;
mod packed;
mod parser;
mod props;
mod regex;
mod source_overlay;
mod source_text;
mod startup;
mod text;
mod value;
mod vm;
mod worker;

pub use parser::{JsxAttributeMark, JsxMark};

use std::path::Path;

pub use census::{Census, CensusGroup};
pub use codecache::CompiledCode;
pub use embed::{DataProperty, DataSurfaceError, HostFunction, Root};
pub use gc::footprint::{Footprint, ListFootprint};
pub use inspect::{
    DebugFiles, FrameView, Origin, Pause, PauseHandle, PauseHandler, PauseReason, Resume,
    SourceFilter, now_ms, pause_handle, pause_soon, paused, same_file, set_breakpoints,
    set_debug_files, set_pause_handler, set_random_seed, set_source_filter, set_time,
};
pub use packed::{
    append_packed_files, append_packed_files_with_diagnostics, open_packed_files,
    packed_diagnostics, read_packed_file, running_packed,
};
pub use source_overlay::{SourceOverlay, with_source_overlay};
pub use startup::write as write_startup_times;

/// Starts reading the files this packed program ran when it was packed, on other threads, so they're ready by the time
/// it runs them. It does nothing in a program that isn't packed. See [`open_packed_files`]. What the program has
/// compiled code for isn't read, as that's loaded instead (see codecache.rs).
pub fn read_packed_files_ahead() {
    let block = codecache::block();
    // What comes with the engine is wanted first, as it's what an app's first file imports.
    let mut order: Vec<std::path::PathBuf> = packed::builtins_run()
        .iter()
        .filter(|name| !block.is_some_and(|block| block.has_builtin(name)))
        .map(|name| modules::builtin_path(name))
        .collect();
    order.extend(
        packed::run_order()
            .into_iter()
            .filter(|path| !block.is_some_and(|block| block.has_file(path))),
    );
    if !order.is_empty() {
        ahead::start(order);
    }
}

/// Says what modules the program is going to give the engine with [`Engine::add_module`], before there's an engine, so that a
/// packed app's can be read ahead along with its files.
pub fn know_modules(modules: &[(&'static str, &'static str)]) {
    for (name, source) in modules {
        modules::know_host_module(name, source);
    }
}
pub use text::Atom;
pub use value::Value;
pub use vm::{Args, Vm};

/// Where Rust's stack is right now, for working out how much of it is in use.
#[inline(never)]
pub(crate) fn stack_position() -> usize {
    let marker = 0u8;
    std::ptr::from_ref(&marker) as usize
}

/// A value in `import.meta.env`. See [`Engine::set_import_meta_env`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaEnvValue {
    Text(String),
    Bool(bool),
}

/// An error from running JavaScript, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsError {
    /// What `console.log` would show for it, which for an error is its stack.
    pub message: String,
}

impl std::fmt::Display for JsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsError {}

/// How much memory the engine has in use, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryUse {
    pub objects: usize,
    pub strings: usize,
    pub shapes: usize,
    pub code: usize,
    pub stack: usize,
}

impl MemoryUse {
    pub fn total(&self) -> usize {
        self.objects + self.strings + self.shapes + self.code + self.stack
    }
}

/// Why some source can't be read, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxProblem {
    pub message: String,
    pub line: u32,
}

/// Whether the engine can read this source, as a script or a module, in TypeScript and JSX or not, without running any of it.
pub fn check_syntax(source: &str) -> Result<(), SyntaxProblem> {
    let memory = ast::AstMemory::new();
    // As much room as a large file could want, on a thread that has it.
    let room = 64 * 1024 * 1024;
    parser::parse(source, &memory, room, false)
        .map(|_| ())
        .map_err(|error| SyntaxProblem {
            message: error.message,
            line: error.line,
        })
}

/// Which files' functions are compiled the first time they're called, rather than with the rest of the file.
///
/// Waiting saves compiling what never runs, which is most of a package's code, but it keeps the file's parsed source
/// until then, and what does run is compiled when it's first called rather than when the file loads. Most of rsui's
/// own runtime and of an app's own files runs by the first frame, so for them it would only move the work and take
/// memory. Even for packages it didn't pay as measured: an app using lodash, date-fns and zod loaded 1% sooner and kept
/// 3.3 MB more, as most of a small package's code runs anyway. So nothing waits unless this is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LazyCompile {
    /// Every function is compiled when the file it's in is loaded.
    #[default]
    Never,
    /// Files with a `node_modules` folder in their path, as they're found on disk or in a packed program.
    NodeModules,
    /// Every file, script and bundled module.
    Always,
}

/// Whether every function in a module that's bundled into the program compiles, as if they'd all been called. A bundled
/// module's functions are compiled when they're first called, and only checked for mistakes then, so tests check them
/// all with this. `params` are what the module's code is wrapped in a function with.
#[doc(hidden)]
pub fn check_compiles(source: &'static str, params: &[&'static str]) -> Result<(), String> {
    let mut engine = Engine::new();
    engine.vm.lazy_compile = LazyCompile::Never;
    let file = engine.vm.intern_str("bundled");
    match engine.vm.compile_wrapped(source, params, file, false) {
        Ok(_) => Ok(()),
        Err(error) => Err(engine.display(error)),
    }
}

/// The parameters a bundled module's code is wrapped in a function with, for [`check_compiles`].
#[doc(hidden)]
pub const MODULE_PARAMS: [&str; 5] = modules::BUILTIN_PARAMS;

/// Where every element of JSX in this source is, in the order they start, for what wants to change them there.
pub fn jsx_marks(source: &str) -> Result<Vec<JsxMark>, SyntaxProblem> {
    parser::jsx_marks(source, 64 * 1024 * 1024).map_err(|error| SyntaxProblem {
        message: error.message,
        line: error.line,
    })
}

/// A JavaScript engine, which holds everything its programs have made.
pub struct Engine {
    ignores_rejections: bool,
    /// What's given each promise that's rejected with nothing to catch it, if something's said it wants them.
    rejections_to: Option<Root>,
    vm: Vm,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self {
            vm: Vm::new(),
            ignores_rejections: false,
            rejections_to: None,
        }
    }

    /// Runs a script: its `var`s and functions are globals, and its `return` is the result, if it has one.
    pub fn eval(&mut self, source: &str, file: &str) -> Result<Value, JsError> {
        let _split = rsui_alloc::split::enter(rsui_alloc::split::JS);
        self.enter();
        let vm = &mut self.vm;
        let file = vm.intern_str(file);
        let lazily = vm.compiles_lazily(None);
        let result = vm.run_script_source(source, file, lazily);
        self.finish(result)
    }

    /// Runs a file as the main module. Imports in it find other files, and packages in `node_modules`.
    /// A main module that waits at its top is waited for, with whatever else is going on running as it would anyway,
    /// so that what it exports is there when this returns, as an app's window needs. It gives up waiting when nothing
    /// is left that could finish it.
    pub fn run_file(&mut self, path: &Path) -> Result<Value, JsError> {
        self.enter();
        let result = self.vm.run_main(path);
        // The program's files have all been read, and what was kept for reading them is no use now.
        ast::release_spare_memory();
        ahead::stop();
        let exports = self.finish(result)?;
        self.run_until(|engine| !engine.vm.main_is_waiting())?;
        Ok(exports)
    }

    /// Runs a file as the main module, as an ES module whatever's in it, as Node does a `.mjs` file. What it imports is
    /// read as it would be otherwise.
    pub fn run_module(&mut self, path: &Path) -> Result<Value, JsError> {
        self.enter();
        let result = self.vm.run_main_module(path);
        ast::release_spare_memory();
        ahead::stop();
        let exports = self.finish(result)?;
        self.run_until(|engine| !engine.vm.main_is_waiting())?;
        Ok(exports)
    }

    fn enter(&mut self) {
        let vm = &mut self.vm;
        vm.nursery.clear();
        vm.stack_base = stack_position();
    }

    fn finish(&mut self, result: Result<Value, Value>) -> Result<Value, JsError> {
        result.map_err(|error| self.error_of(error))
    }

    fn error_of(&mut self, error: Value) -> JsError {
        JsError {
            message: builtins::inspect_value(&mut self.vm, error),
        }
    }

    /// Runs everything that's waiting: finished promises, and so on. It runs nothing if nothing's waiting.
    pub fn run_jobs(&mut self) -> Result<(), JsError> {
        let _split = rsui_alloc::split::enter(rsui_alloc::split::JS);
        self.enter();
        self.vm.collect_background();
        let result = self.vm.run_jobs().map(|()| Value::UNDEFINED);
        self.finish(result)?;
        self.check_rejections()
    }

    /// Has a promise that's rejected with nothing to catch it not be an error from [`run_jobs`](Self::run_jobs), which is
    /// for code that means to leave them that way. An app shouldn't, as that's how it hears about them.
    pub fn ignore_unhandled_rejections(&mut self) {
        self.ignores_rejections = true;
    }

    /// Has each promise that's rejected with nothing to catch it, once everything that was waiting has run, be given to
    /// `handler`, as `handler(reason, promise)`, rather than be an error from [`run_jobs`](Self::run_jobs). That's how a
    /// browser has it: the page hears about it with `unhandledrejection`, and it's logged, and the page goes on. What
    /// `handler` throws is the error then.
    pub fn report_rejections_to(&mut self, handler: Value) {
        self.rejections_to = Some(self.vm.root(handler));
    }

    /// Has the engine write down what each file it compiles from now on compiles to, for [`take_compiled_code`]
    /// (Self::take_compiled_code): for packing an app, whose program then loads it rather than compiling its files again
    /// (see [`append_packed_files`]). Files compiled lazily, for a debugger, or through a source filter aren't.
    pub fn keep_compiled_code(&mut self) {
        self.vm.keep_compiled_code();
    }

    /// What's been written down since [`keep_compiled_code`](Self::keep_compiled_code), if it was asked for.
    pub fn take_compiled_code(&mut self) -> Option<CompiledCode> {
        self.vm.take_compiled_code()
    }

    /// Has every file this engine compiles from now on go through the code cache's format and back, checked to be the
    /// same, with what was read back being what runs: for testing the cache. `RSUI_CODE_CACHE_CHECK=1` does this for
    /// every engine.
    #[doc(hidden)]
    pub fn check_code_cache(&mut self, on: bool) {
        self.vm.check_code_cache(on);
    }

    /// Which files have their functions compiled the first time they're called, rather than all at once with the
    /// code they're in. See [`LazyCompile`].
    pub fn set_lazy_compile(&mut self, which: LazyCompile) {
        self.vm.lazy_compile = which;
    }

    /// Every file's functions compiled when they're first called, or none's, for measuring the difference and for
    /// tests. See [`set_lazy_compile`](Self::set_lazy_compile).
    #[doc(hidden)]
    pub fn compile_lazily(&mut self, lazily: bool) {
        self.vm.lazy_compile = if lazily {
            LazyCompile::Always
        } else {
            LazyCompile::Never
        };
    }

    fn check_rejections(&mut self) -> Result<(), JsError> {
        // The main module going wrong after it waited at its top is the program going wrong, whatever else is ignored.
        if let Some((error, promise)) = self.vm.modules.take_main_failure() {
            self.vm
                .unhandled_rejections
                .retain(|found| *found != promise);
            return Err(self.error_of(error));
        }
        if self.ignores_rejections {
            self.vm.unhandled_rejections.clear();
        }
        if let Some(handler) = self.rejections_to.as_ref().map(Root::get) {
            // What the handler does can leave more of them, which it hears about too.
            while !self.vm.unhandled_rejections.is_empty() {
                // Each is held while the ones before it are told about, as what they run can collect.
                let rejected: Vec<Root> = std::mem::take(&mut self.vm.unhandled_rejections)
                    .into_iter()
                    .map(|promise| self.vm.root(promise))
                    .collect();
                for promise in rejected {
                    let promise = promise.get();
                    let reason = match &self.vm.objects[promise.index()].kind {
                        object::Kind::Promise(data) => data.value,
                        _ => Value::UNDEFINED,
                    };
                    let told = self.vm.call(handler, Value::UNDEFINED, &[reason, promise]);
                    self.finish(told)?;
                }
                let ran = self.vm.run_jobs().map(|()| Value::UNDEFINED);
                self.finish(ran)?;
            }
            return Ok(());
        }
        if let Some(first) = self.vm.unhandled_rejections.first().copied() {
            self.vm.unhandled_rejections.clear();
            let promise_value = match &self.vm.objects[first.index()].kind {
                object::Kind::Promise(data) => data.value,
                _ => Value::UNDEFINED,
            };
            return Err(self.error_of(promise_value));
        }
        Ok(())
    }

    /// Whether there's anything for [`run_jobs`](Self::run_jobs) or [`run_timers`](Self::run_timers) to do.
    pub fn has_pending(&self) -> bool {
        !self.vm.jobs.is_empty()
            || !self.vm.cleanups.is_empty()
            || !self.vm.timers.is_empty()
            || !self.vm.unhandled_rejections.is_empty()
            || self.vm.background.any_ready()
    }

    /// Whether anything's going on on another thread, such as a file being read.
    pub fn is_working(&self) -> bool {
        self.vm.background.any_waiting()
    }

    /// Whether anything's going on on another thread that's going to finish by itself, unlike a socket waiting to hear something.
    pub fn is_busy(&self) -> bool {
        self.vm.background.any_busy()
    }

    /// What a worker has that a program doesn't.
    pub(crate) fn load_worker_globals(&mut self) -> Result<(), JsError> {
        self.enter();
        let loaded = self.vm.load_host_module("worker_threads");
        self.finish(loaded).map(|_| ())
    }

    /// Whether something's ready to be run right now, rather than later.
    pub(crate) fn has_ready_work(&self) -> bool {
        !self.vm.jobs.is_empty()
            || !self.vm.cleanups.is_empty()
            || !self.vm.unhandled_rejections.is_empty()
            || self.vm.background.any_ready()
    }

    /// Asks to be called when something on another thread has finished, such as a file being read, so that
    /// [`run_jobs`](Self::run_jobs) can pass that on. `waker` runs on that thread, so it should only signal this one.
    pub fn set_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        self.vm.background.set_waker(waker);
    }

    /// Whether a timer that hasn't been unref'd is waiting, which, as in Node, is what keeps a program going for a
    /// timer: one that's been unref'd still runs if something else does.
    pub fn timers_keep_alive(&self) -> bool {
        self.vm.timers_keep_alive()
    }

    /// How many milliseconds until the next timer's due, if there's one, without running any.
    pub fn next_timer_in(&self) -> Option<f64> {
        self.vm
            .timers
            .iter()
            .map(|timer| timer.at)
            .min_by(f64::total_cmp)
            .map(|at| (at - Vm::now_ms()).max(0.0))
    }

    /// Runs the timers that are due, and says how many milliseconds until the next, if there's one waiting.
    pub fn run_timers(&mut self) -> Result<Option<f64>, JsError> {
        let _split = rsui_alloc::split::enter(rsui_alloc::split::JS);
        self.enter();
        let now = Vm::now_ms();
        match self.vm.run_timers(now) {
            Ok(next) => {
                self.check_rejections()?;
                Ok(next.map(|at| (at - Vm::now_ms()).max(0.0)))
            }
            Err(error) => Err(self.error_of(error)),
        }
    }

    /// Runs everything to the end: jobs and timers, waiting for the timers.
    pub fn run_until_idle(&mut self) -> Result<(), JsError> {
        self.run_until(|_| false)
    }

    /// Runs jobs and timers, waiting for them, until `done` says so or there's nothing left that could happen.
    fn run_until(&mut self, done: impl Fn(&Self) -> bool) -> Result<(), JsError> {
        loop {
            if done(self) {
                return Ok(());
            }
            self.run_jobs()?;
            if done(self) {
                return Ok(());
            }
            let wait = self.run_timers()?;
            self.run_jobs()?;
            let working = self.vm.background.any_waiting();
            // Timers that have been unref'd still run while something else keeps things going, but don't by themselves.
            let wait = wait.filter(|_| working || self.vm.timers_keep_alive());
            // Waiting to be woken is only as exact as the system's clock, which ticks every 15.6 ms or so. Sleeping is
            // exact, so the last part of the wait for a timer is slept a millisecond at a time.
            const TICK: f64 = 16.0;
            match wait {
                // Something on another thread could finish at any moment, so that can't be slept through.
                Some(wait) if working && wait > TICK => {
                    let longest = std::time::Duration::from_secs_f64((wait - TICK) / 1000.0);
                    self.vm.background.wait_for_something(Some(longest));
                }
                Some(wait) if working => {
                    std::thread::sleep(std::time::Duration::from_secs_f64(
                        (wait / 1000.0).min(0.001),
                    ));
                }
                Some(wait) => {
                    if wait > 0.0 {
                        std::thread::sleep(std::time::Duration::from_secs_f64(wait / 1000.0));
                    }
                }
                None if working => self.vm.background.wait_for_something(None),
                // A registry's callback runs in the turn after its target went.
                None if !self.vm.cleanups.is_empty() => {}
                None => return Ok(()),
            }
        }
    }

    /// Sends what `console.log` prints somewhere else. It goes to the standard output otherwise.
    pub fn set_printer(&mut self, printer: impl FnMut(&str) + 'static) {
        self.vm.printer = Box::new(printer);
    }

    /// Makes a value something Rust can show.
    pub fn display(&mut self, value: Value) -> String {
        if value.is_string() {
            return self.vm.text_of(value).to_rust();
        }
        builtins::inspect_value(&mut self.vm, value)
    }

    /// Frees everything nothing can reach any more. The engine does this by itself as needed.
    pub fn collect_garbage(&mut self) {
        self.vm.nursery.clear();
        self.vm.run_mark = 0;
        self.vm.top = 0;
        self.vm.collect_all_garbage();
    }

    /// What an app does once it's been quiet a while: frees everything nothing can reach, then gives back
    /// the pages of the engine's lists with nothing left on them and shrinks objects' storage to fit, which costs time later
    /// if the app gets busy again, so ordinary collections don't.
    pub fn collect_garbage_when_quiet(&mut self) {
        let before = gc::giveback::FORCE.with(|on| on.replace(true));
        self.collect_garbage();
        gc::giveback::FORCE.with(|on| on.set(before));
    }

    /// Says how much of Rust's stack the engine can use, for a thread that has more than the usual megabyte.
    pub fn set_stack_limit(&mut self, bytes: usize) {
        self.vm.stack_limit = bytes;
    }

    /// Sets a variable in `process.env`, which starts out empty.
    /// Gives every module an `import.meta.env` with these in it, as a bundler like Vite does for an app's code: text,
    /// or `true` or `false`. Node gives modules none, so there's none unless this is called. It's the same object for
    /// every module, and it's what modules that haven't asked for `import.meta` yet see.
    pub fn set_import_meta_env(&mut self, entries: &[(&str, MetaEnvValue)]) {
        let env = self.vm.new_plain();
        for (key, value) in entries {
            let atom = self.vm.intern_str(key);
            let value = match value {
                MetaEnvValue::Text(text) => self.vm.new_str(text),
                MetaEnvValue::Bool(on) => Value::bool(*on),
            };
            self.vm.define(env.handle(), atom, value, object::PLAIN);
        }
        self.vm.modules.meta_env = Some(env);
    }

    pub fn set_env(&mut self, key: &str, value: &str) {
        let atom = self.vm.intern_str(key);
        let value = self.vm.new_str(value);
        let env = self.vm.realm.process_env;
        self.vm.define(env, atom, value, object::PLAIN);
    }

    /// How many times garbage has been collected.
    pub fn collections(&self) -> u32 {
        self.vm.gc_count
    }

    /// What the engine's lists of objects, strings and cells take from the system, and how many of their places are free.
    #[doc(hidden)]
    pub fn footprint(&self) -> Footprint {
        self.vm.footprint()
    }

    /// How much memory the engine has in use.
    pub fn memory(&self) -> MemoryUse {
        self.vm.memory()
    }

    /// Where the JIT's code for each function it has compiled is, for a native profile to say which function and
    /// instruction a sample in it was at.
    #[cfg(feature = "jit")]
    pub fn jit_code(&self) -> Vec<JitCode> {
        jit::Jit::code_map(&self.vm)
    }
}

/// One function's compiled code (see [`Engine::jit_code`]): its name and where it's from, the addresses it's
/// between, and for each instruction, where its code and its cold paths start (from `start`) and what it is.
#[cfg(feature = "jit")]
pub struct JitCode {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub ops: Vec<(u32, u32, String)>,
}

#[cfg(test)]
mod equivalent_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_spec;
