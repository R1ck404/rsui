//! What a Rust program uses to work with the engine: reading and making values, calling functions, and holding on to
//! values between calls.
//!
//! A [`Value`] on its own is only safe until JavaScript next runs, as the collector doesn't know Rust has it. To keep one
//! for longer, make it a [`Root`].

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::object::*;
use crate::text::{Atom, AtomKind};
use crate::value::Value;
use crate::vm::{Args, Vm};

#[path = "data_surface.rs"]
mod data_surface;
pub use data_surface::{DataProperty, DataSurfaceError};
pub(crate) use data_surface::SelectorIntrinsic;

/// What a Rust function looks like to JavaScript.
pub type HostFunction = fn(&mut Vm, Args) -> Result<Value, Value>;

thread_local! {
    /// Host functions made while `RSUI_SPLIT` is set, each with its part, by the number [`timed_host`] is made with.
    static TIMED_HOSTS: RefCell<Vec<(HostFunction, u16)>> = const { RefCell::new(Vec::new()) };
}

/// A host function, with its time charged to its own part.
fn timed_host(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let at = vm.native_data_of(&args).as_number() as usize;
    let (function, part) = TIMED_HOSTS.with(|hosts| hosts.borrow()[at]);
    let _split = rsui_alloc::split::enter(part);
    function(vm, args)
}

#[derive(Default)]
pub(crate) struct Held {
    /// Each value, with how many `Root`s there are of it. A free place has a count of zero.
    slots: RefCell<Vec<(Value, u32)>>,
    free: RefCell<Vec<u32>>,
}

impl Held {
    /// Every value being held, for the collector.
    pub(crate) fn each(&self, mut visit: impl FnMut(Value)) {
        for (value, count) in self.slots.borrow().iter() {
            if *count > 0 {
                visit(*value);
            }
        }
    }
}

/// A value that stays alive for as long as this does. Cloning one is cheap.
pub struct Root {
    table: Rc<Held>,
    slot: u32,
}

impl Clone for Root {
    fn clone(&self) -> Self {
        self.table.slots.borrow_mut()[self.slot as usize].1 += 1;
        Self {
            table: self.table.clone(),
            slot: self.slot,
        }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let mut slots = self.table.slots.borrow_mut();
        let entry = &mut slots[self.slot as usize];
        entry.1 -= 1;
        if entry.1 == 0 {
            entry.0 = Value::UNDEFINED;
            self.table.free.borrow_mut().push(self.slot);
        }
    }
}

impl Root {
    /// The value being held.
    #[inline]
    pub fn get(&self) -> Value {
        self.table.slots.borrow()[self.slot as usize].0
    }
}

impl std::fmt::Debug for Root {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Root({:?})", self.get())
    }
}

impl Args {
    /// The argument at this position, or undefined if there aren't that many.
    #[inline]
    pub fn get(&self, vm: &Vm, index: usize) -> Value {
        vm.arg(self, index)
    }

    /// What `this` is.
    #[inline]
    pub fn this(&self) -> Value {
        self.this
    }

    /// How many arguments there are.
    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }
}

impl Vm {
    /// The files that have been run as modules so far, for a host function that wants to know what the program is made of.
    pub fn loaded_files(&self) -> Vec<PathBuf> {
        self.modules.loaded_files()
    }

    /// How many [`loaded_files`](Self::loaded_files) there are, without making the list.
    pub fn loaded_file_count(&self) -> usize {
        self.modules.loaded_file_count()
    }

    /// Where `require` would find `name` from `dir`, with packages' `exports` read with these conditions as well as the usual
    /// ones, as a bundler finds a package's stylesheet with `style`. It's `None` if there's nothing there.
    pub fn resolve_with_conditions(&mut self, dir: &std::path::Path, name: &str, conditions: &[&str]) -> Option<PathBuf> {
        let usual = std::mem::replace(
            &mut self.modules.conditions,
            conditions.iter().map(|one| (*one).to_owned()).collect(),
        );
        let found = self.resolve_file(dir, name, false);
        self.modules.conditions = usual;
        found.ok().map(|found| crate::modules::canonical(&found))
    }

    /// Holds on to a value for as long as the [`Root`] lasts.
    pub fn root(&mut self, value: Value) -> Root {
        let slot = if let Some(slot) = self.held.free.borrow_mut().pop() {
            self.held.slots.borrow_mut()[slot as usize] = (value, 1);
            slot
        } else {
            let mut slots = self.held.slots.borrow_mut();
            slots.push((value, 1));
            (slots.len() - 1) as u32
        };
        Root {
            table: self.held.clone(),
            slot,
        }
    }

    /// The name of a property, which is quicker to use than its text. It stays for as long as the engine does, so it
    /// can be kept. An atom from anywhere else, such as [`properties`](Self::properties), is only good until JavaScript
    /// next runs, as a collection then can free one that nothing uses, and give its number to another.
    pub fn atom(&mut self, name: &str) -> Atom {
        self.intern_str(name)
    }

    /// Goes up whenever a collection frees atoms, so whatever Rust remembers by an atom's [`number`](Atom::number) has
    /// to be forgotten when this changes, other than what's about atoms from [`atom`](Self::atom).
    pub fn atom_generation(&self) -> u32 {
        self.atoms.generation
    }

    /// The text of a property's name.
    pub fn atom_name(&self, atom: Atom) -> String {
        self.atom_text(atom)
    }

    /// Reads a property, as `object.name` does.
    pub fn get_property(&mut self, object: Value, name: Atom) -> Result<Value, Value> {
        self.get_by_atom(object, name)
    }

    /// Writes a property, as `object.name = value` does.
    pub fn set_property(&mut self, object: Value, name: Atom, value: Value) -> Result<(), Value> {
        self.set(object, name, value)
    }

    /// Calls a function. Whatever it gives back is safe until this comes back to the top, as it's on the list of what
    /// natives are holding, unless [`release_new_values`](Self::release_new_values) gets rid of it first.
    pub fn call_function(
        &mut self,
        function: Value,
        this: Value,
        args: &[Value],
    ) -> Result<Value, Value> {
        let _split = rsui_alloc::split::enter(rsui_alloc::split::JS);
        self.call(function, this, args)
    }

    /// Where the list of what natives are holding has got to, for
    /// [`release_new_values`](Self::release_new_values).
    #[inline]
    pub fn new_values_mark(&self) -> usize {
        self.nursery.len()
    }

    /// Stops holding what was made since this mark, once Rust has finished with those values.
    #[inline]
    pub fn release_new_values(&mut self, mark: usize) {
        self.nursery.truncate(mark);
    }

    /// A new string.
    pub fn new_string(&mut self, text: &str) -> Value {
        self.new_str(text)
    }

    /// A new object with no properties of its own.
    pub fn new_object(&mut self) -> Value {
        self.new_plain()
    }

    /// Runs something on another thread, and gives a promise for how it turns out: text, `null` if there's nothing to say,
    /// or an error. The host passes on what's finished by calling [`Engine::run_jobs`](crate::Engine::run_jobs).
    pub fn later(
        &mut self,
        job: impl FnOnce() -> Result<Option<String>, String> + Send + 'static,
    ) -> Value {
        self.in_background(move || match job() {
            Ok(Some(text)) => crate::background::Outcome::Text(text),
            Ok(None) => crate::background::Outcome::Null,
            Err(reason) => crate::background::Outcome::Failed(reason),
        })
    }

    /// A function that JavaScript can call.
    pub fn host_function(&mut self, name: &str, length: u8, function: HostFunction) -> Value {
        // With `RSUI_SPLIT` set, each host function's time is its own part: see `split.rs` in rsui-alloc.
        if rsui_alloc::split::on() {
            let part = rsui_alloc::split::named(&format!("host:{name}"));
            let at = TIMED_HOSTS.with(|hosts| {
                let mut hosts = hosts.borrow_mut();
                hosts.push((function, part));
                hosts.len() - 1
            });
            let name = self.intern_str(name);
            return self.new_native(name, length, timed_host, Value::number(at as f64), false);
        }
        let name = self.intern_str(name);
        self.new_native(name, length, function, Value::UNDEFINED, false)
    }

    /// Runs a module from [`Engine::add_module`](crate::Engine::add_module) if nothing has yet, and gives what it exports.
    pub fn load_host_module(&mut self, name: &str) -> Result<Value, Value> {
        match self.load_builtin_module(name)? {
            Some(module) => self.exports_of(module),
            None => Ok(Value::UNDEFINED),
        }
    }

    /// Makes a global variable.
    pub fn set_global(&mut self, name: &str, value: Value) {
        let name = self.intern_str(name);
        let global = self.global;
        self.define(global, name, value, HIDDEN);
    }

    /// Reads a global variable. It's undefined if there's no such thing.
    pub fn get_global(&mut self, name: &str) -> Result<Value, Value> {
        let name = self.intern_str(name);
        self.get(Value::object(self.global), name)
    }

    /// Whether this can be called.
    #[inline]
    pub fn is_function(&self, value: Value) -> bool {
        self.is_callable(value)
    }

    /// The text of a string, without copying it if that can be helped.
    pub fn string_text(&self, value: Value) -> Option<Cow<'_, str>> {
        if !value.is_string() {
            return None;
        }
        let text = self.text_of(value);
        Some(match text.ascii() {
            // ASCII is always valid.
            Some(bytes) => String::from_utf8_lossy(bytes),
            None => Cow::Owned(text.to_rust()),
        })
    }

    /// Finds packages as a bundler for a browser does, with `exports`' `browser` condition and the `browser` field of a
    /// `package.json`, for a program whose code is written for a page, so that a library's build for browsers is the one it gets.
    pub fn set_browser_packages(&mut self, on: bool) {
        self.modules.browser = on;
    }

    /// The modules that come with the engine, or that the program gave it, that have been started so far, in order.
    pub fn loaded_builtins(&self) -> Vec<String> {
        self.modules.loaded_builtins()
    }

    /// What `String(value)` would give for a number or a string, without running any of the program's code.
    pub fn plain_text(&mut self, value: Value) -> Option<String> {
        if value.is_string() {
            return Some(self.text_of(value).to_rust());
        }
        if value.is_int() {
            return Some(value.as_int().to_string());
        }
        if value.is_number() {
            return Some(crate::convert::number_to_string(value.as_number()));
        }
        None
    }

    /// Where the bytes an `ArrayBuffer` holds are, or the part of one that a typed array or `DataView` looks at, as the buffer
    /// and the range in it. `None` for anything else, or a buffer that's been given away.
    fn byte_range(&self, value: Value) -> Option<(Value, std::ops::Range<usize>)> {
        if !value.is_object() {
            return None;
        }
        let (buffer, start, length) = match &self.objects[value.index()].kind {
            Kind::ArrayBuffer(data) => (value, 0, data.bytes.len()),
            Kind::TypedArray(data) => (data.buffer, data.offset, self.typed_length(data) * data.element.size()),
            Kind::DataView(view) => {
                let total = self.buffer_of(view.buffer)?.bytes.len();
                let length = if view.follows_buffer { total.saturating_sub(view.offset) } else { view.length };
                (view.buffer, view.offset, length)
            }
            _ => return None,
        };
        let data = self.buffer_of(buffer)?;
        if data.detached || start + length > data.bytes.len() {
            return None;
        }
        Some((buffer, start..start + length))
    }

    /// The bytes an `ArrayBuffer` holds, or that a typed array or `DataView` looks at, which is how a host function reads what
    /// a program put in one without copying it.
    pub fn bytes(&self, value: Value) -> Option<&[u8]> {
        let (buffer, range) = self.byte_range(value)?;
        Some(&self.buffer_of(buffer)?.bytes[range])
    }

    /// The same, to change them, as a host function that fills in what the program reads does.
    pub fn bytes_mut(&mut self, value: Value) -> Option<&mut [u8]> {
        let (buffer, range) = self.byte_range(value)?;
        match &mut self.objects[buffer.index()].kind {
            Kind::ArrayBuffer(data) => Some(&mut data.bytes[range]),
            _ => None,
        }
    }

    /// A new `ArrayBuffer` holding these bytes.
    pub fn new_buffer(&mut self, bytes: Vec<u8>) -> Value {
        self.new_array_buffer(bytes)
    }

    /// The elements of an array. A hole reads as a value that's none of the kinds there are, so skip what isn't wanted.
    pub fn array_elements(&self, value: Value) -> Option<&[Value]> {
        if !value.is_object() {
            return None;
        }
        match &self.objects[value.index()].kind {
            Kind::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The same as [`properties`](Self::properties), into a list that can be used again. This runs for
    /// everything drawn, and making a list each time was a good part of what that costs.
    pub fn properties_into(
        &mut self,
        object: Value,
        out: &mut Vec<(Atom, Value)>,
    ) -> Result<(), Value> {
        out.clear();
        if !object.is_object() {
            return Ok(());
        }
        let found = &self.objects[object.index()];
        if matches!(found.kind, Kind::Plain) && self.shapes.is_all_plain(found.shape) {
            let slots = found.slots();
            let atoms = &self.atoms;
            let backwards = self.shapes.visit_entries(found.shape, |key, slot| {
                if atoms.kind_of(key) == AtomKind::Name {
                    out.push((key, slots[slot as usize]));
                }
            });
            if backwards {
                out.reverse();
            }
            return Ok(());
        }
        *out = self.properties(object)?;
        Ok(())
    }

    /// An object's own properties that a loop would find, with their values.
    pub fn properties(&mut self, object: Value) -> Result<Vec<(Atom, Value)>, Value> {
        if !object.is_object() {
            return Ok(Vec::new());
        }
        let found = &self.objects[object.index()];
        if matches!(found.kind, Kind::Plain) && self.shapes.is_all_plain(found.shape) {
            let entries = self.shapes.entries(found.shape);
            let slots = self.objects[object.index()].slots();
            return Ok(entries
                .into_iter()
                .filter(|(key, _, _)| self.atoms.kind_of(*key) == AtomKind::Name)
                .map(|(key, slot, _)| (key, slots[slot as usize]))
                .collect());
        }
        let names = self.own_enumerable_names(object.handle());
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let value = self.get(object, name)?;
            out.push((name, value));
        }
        Ok(out)
    }

    /// Whether two values are the same, as `Object.is` says.
    pub fn same(&self, a: Value, b: Value) -> bool {
        self.same_value(a, b)
    }

    /// Whether two objects have the same properties holding the same things, as a component's props are compared. This
    /// never runs any of the program's code, and objects that were built differently count as different.
    pub fn shallow_equal(&self, a: Value, b: Value) -> bool {
        if a == b {
            return true;
        }
        if !a.is_object() || !b.is_object() {
            return self.same_value(a, b);
        }
        let (first, second) = (&self.objects[a.index()], &self.objects[b.index()]);
        if !matches!(first.kind, Kind::Plain)
            || !matches!(second.kind, Kind::Plain)
            || first.shape != second.shape
            || self.shapes.is_dict(first.shape)
        {
            return false;
        }
        first
            .slots()
            .iter()
            .zip(second.slots())
            .all(|(x, y)| x == y || self.same_value(*x, *y))
    }

    /// Whether nothing an object holds is an object or a function itself, so that nothing it holds can say something else later
    /// without being given anew. Something that isn't an object holds nothing. An object that wasn't built plainly isn't known to.
    pub fn holds_no_objects(&self, value: Value) -> bool {
        if !value.is_object() {
            return true;
        }
        let object = &self.objects[value.index()];
        matches!(object.kind, Kind::Plain)
            && !self.shapes.is_dict(object.shape)
            && object.slots().iter().all(|slot| !slot.is_object())
    }

    /// What an error says, with its stack if it has one.
    pub fn error_text(&mut self, error: Value) -> String {
        crate::builtins::inspect_value(self, error)
    }
}

impl crate::Engine {
    /// The engine itself, for what's in this module.
    #[inline]
    pub fn vm(&mut self) -> &mut Vm {
        &mut self.vm
    }

    /// Adds a module that importing this name gives, instead of looking for it in `node_modules`. It's written as
    /// CommonJS, and only compiled if something asks for it.
    pub fn add_module(&mut self, name: &'static str, source: &'static str) {
        self.vm.modules.host.push((name, source));
    }

    /// The modules that come with the engine, or that [`add_module`](Self::add_module) gave it, that have been run, in the
    /// order they were.
    pub fn loaded_builtins(&self) -> Vec<String> {
        self.vm.modules.loaded_builtins()
    }

    /// The files that have been run as modules.
    pub fn loaded_files(&self) -> Vec<PathBuf> {
        self.vm.modules.loaded_files()
    }

    /// Forgets which files have been run, so running one again reads it all again. What comes with the engine, and what
    /// [`add_module`](Self::add_module) gave it, stay as they are.
    pub fn forget_files(&mut self) {
        self.vm.modules.forget_files();
    }

    /// Turns an error into text.
    pub fn error_message(&mut self, error: Value) -> crate::JsError {
        self.error_of(error)
    }
}
